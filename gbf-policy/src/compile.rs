//! Compile-request and resolved-policy schema.

use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

use gbf_foundation::{BlobRef, CompileProfileId, Hash256, TargetProfileId};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_hw::target::TargetProfile;
use serde::de::Error as _;
use serde::ser::SerializeSeq;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::RuntimeChromeBudget;
use crate::canonical::domain_hash;
use crate::diagnostics::{ReductionSiteId, TraceProbeId};
use crate::objective::{CompileObjective, RiskPolicy};
use crate::repair::{RepairPolicy, RepairPolicyProfile, RepairProposalId, RepairReason};

pub use gbf_foundation::{EvidenceRef, FieldPath};

pub const BRINGUP_COMPILE_PROFILE_ID: &str = "Bringup";
pub const DEFAULT_COMPILE_PROFILE_ID: &str = "Default";
pub const TRACE_COMPILE_PROFILE_ID: &str = "Trace";
pub const RECOVERY_COMPILE_PROFILE_ID: &str = "Recovery";

pub const BRINGUP_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/bringup.profile.toml");
pub const DEFAULT_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/default.profile.toml");
pub const TRACE_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/trace.profile.toml");
pub const RECOVERY_COMPILE_PROFILE_TOML: &str =
    include_str!("../fixtures/compile-profiles/recovery.profile.toml");

/// RFC-shaped domain separator for the canonical `CompileProfileSpec` defaults hash.
///
/// The preimage is this byte string followed by canonical JSON for the profile
/// spec with `defaults_hash` zeroed. The NUL suffix prevents accidental
/// concatenation ambiguity with the first JSON byte.
pub const COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN: &[u8] =
    b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:2.0.0\0";

pub const COMPILE_PROFILE_SPEC_VERSION: &str = "2.0.0";
pub const COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE: &str =
    "POLICY-PROFILE-SCHEMA-VERSION-UNSUPPORTED";
pub const PROFILE_SPEC_V2_LOADED_EVENT: &str = "gbf_policy.profile_spec.v2_loaded";
pub const PROFILE_SPEC_V1_REJECTED_EVENT: &str = "gbf_policy.profile_spec.v1_rejected";
pub const PROFILE_SPEC_V2_INVARIANT_FAILURE_EVENT: &str =
    "gbf_policy.profile_spec.v2_invariant_failure";

const PROFILE_SPEC_INLINE_FIXTURE_PATH: &str = "inline";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SequenceSemanticsRef {
    #[default]
    Unspecified,
    LinearState,
    BoundedKv,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SelectorPath(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileRequest {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub calibration_set_ref: CalibrationSetRef,
    pub required_features: BTreeSet<CompilerFeature>,
    pub constraint_overrides: Option<CompileKnobOverrides>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

pub type ArtifactRef = BlobRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileInvocationInputs {
    pub artifact_ref: ArtifactRef,
    pub compile_request: CompileRequest,
    pub target_profile: TargetProfile,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedCompilePolicy {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub range_caps: RangeCapsSpec,
    pub observation_caps: ObservationProfileCaps,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub knobs: CompileKnobs,
    pub repair: RepairPolicy,
    pub provenance: PolicyProvenance,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileProfileSpec {
    pub schema_version: String,
    pub id: CompileProfileId,
    pub defaults_hash: Hash256,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub range_caps: RangeCapsSpec,
    pub observation_caps: ObservationProfileCaps,
    pub repair_policy: RepairPolicy,
    pub risk_policy: RiskPolicy,
    pub knob_defaults: CompileKnobPartialValues,
    pub knob_bounds: CompileKnobPartialBounds,
    pub locks: KnobLockSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RenormStrategyPolicy {
    ExactPostBoundaryOnly,
    DynamicMargin { margin_q16_16: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeCapsSpec {
    pub profile_chunk_max: u16,
    pub profile_tile_max: u16,
    pub profile_tile_min: u16,
    pub renorm_strategy: RenormStrategyPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationProfileCaps {
    pub required_max: Option<u16>,
    pub important_max: u16,
    pub diagnostic_max: u16,
    pub best_effort_max: u16,
}

impl ObservationProfileCaps {
    #[must_use]
    pub const fn default_v2() -> Self {
        Self {
            required_max: None,
            important_max: 256,
            diagnostic_max: 256,
            best_effort_max: 256,
        }
    }
}

impl RangeCapsSpec {
    #[must_use]
    pub const fn default_v2() -> Self {
        Self {
            profile_chunk_max: 256,
            profile_tile_max: 256,
            profile_tile_min: 16,
            renorm_strategy: RenormStrategyPolicy::ExactPostBoundaryOnly,
        }
    }
}

#[derive(Debug)]
pub enum CompileProfileSpecLoadError {
    Toml(toml::de::Error),
    UnsupportedSchemaVersion {
        code: &'static str,
        actual: String,
        expected: &'static str,
    },
    MissingRequiredField {
        field: &'static str,
    },
    InvalidInvariant {
        invariant: &'static str,
        field: &'static str,
        value: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitialKnobsResolveError {
    UnknownProfile {
        profile: CompileProfileId,
    },
    MissingDefault {
        profile: CompileProfileId,
        field: &'static str,
    },
    MissingBound {
        profile: CompileProfileId,
        field: &'static str,
    },
}

impl CompileProfileSpecLoadError {
    #[must_use]
    pub const fn code(&self) -> Option<&'static str> {
        match self {
            Self::UnsupportedSchemaVersion { code, .. } => Some(code),
            _ => None,
        }
    }
}

impl fmt::Display for CompileProfileSpecLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Toml(err) => write!(f, "{err}"),
            Self::UnsupportedSchemaVersion {
                code,
                actual,
                expected,
            } => write!(
                f,
                "{code}: compile profile schema version {actual} is unsupported; expected {expected}"
            ),
            Self::MissingRequiredField { field } => {
                write!(f, "compile profile missing required field {field}")
            }
            Self::InvalidInvariant {
                invariant,
                field,
                value,
            } => write!(
                f,
                "compile profile invariant {invariant} failed for {field}={value}"
            ),
        }
    }
}

impl Error for CompileProfileSpecLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Toml(err) => Some(err),
            _ => None,
        }
    }
}

impl fmt::Display for InitialKnobsResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownProfile { profile } => {
                write!(f, "unknown compile profile {}", profile.as_str())
            }
            Self::MissingDefault { profile, field } => {
                write!(
                    f,
                    "compile profile {} is missing knob default {field}",
                    profile.as_str()
                )
            }
            Self::MissingBound { profile, field } => {
                write!(
                    f,
                    "compile profile {} is missing knob bound {field}",
                    profile.as_str()
                )
            }
        }
    }
}

impl Error for InitialKnobsResolveError {}

impl From<toml::de::Error> for CompileProfileSpecLoadError {
    fn from(err: toml::de::Error) -> Self {
        Self::Toml(err)
    }
}

pub fn compile_profile_defaults_hash(
    spec: &CompileProfileSpec,
) -> Result<Hash256, serde_json::Error> {
    let canonical_bytes = compile_profile_defaults_hash_canonical_json_bytes(spec)?;
    let mut hasher = Sha256::new();
    hasher.update(COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN);
    hasher.update(canonical_bytes);
    let digest = hasher.finalize();
    Ok(Hash256::from_bytes(digest.into()))
}

fn compile_profile_defaults_hash_canonical_json_bytes(
    spec: &CompileProfileSpec,
) -> Result<Vec<u8>, serde_json::Error> {
    let mut hashable = spec.clone();
    hashable.defaults_hash = Hash256::ZERO;
    let value = serde_json::to_value(&hashable)?;
    let mut canonical_bytes = Vec::new();
    write_canonical_json_value(&value, &mut canonical_bytes)?;
    Ok(canonical_bytes)
}

fn write_canonical_json_value(
    value: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_writer(out, value)?,
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_json_value(item, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(entries) => {
            out.push(b'{');
            let mut keys = entries.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_json_value(&entries[key], out)?;
            }
            out.push(b'}');
        }
    }
    Ok(())
}

pub fn load_compile_profile_spec(
    toml_source: &str,
) -> Result<CompileProfileSpec, CompileProfileSpecLoadError> {
    let value = toml::from_str::<toml::Value>(toml_source)?;
    let table = value
        .as_table()
        .ok_or_else(|| toml::de::Error::custom("compile profile root must be a TOML table"))?;
    let has_range_caps = table.contains_key("range_caps");
    let has_observation_caps = table.contains_key("observation_caps");
    match table.get("schema_version") {
        Some(actual) => {
            let actual = schema_version_literal(actual);
            if actual != COMPILE_PROFILE_SPEC_VERSION {
                return Err(unsupported_schema_version_error(
                    actual,
                    "unsupported_schema_version",
                ));
            }
        }
        None if !has_range_caps && !has_observation_caps => {
            return Err(unsupported_schema_version_error(
                "1.0.0",
                "legacy_v1_missing_schema_version_and_caps",
            ));
        }
        None => {
            return Err(CompileProfileSpecLoadError::MissingRequiredField {
                field: "schema_version",
            });
        }
    }
    if !has_range_caps {
        return Err(CompileProfileSpecLoadError::MissingRequiredField {
            field: "range_caps",
        });
    }
    if !has_observation_caps {
        return Err(CompileProfileSpecLoadError::MissingRequiredField {
            field: "observation_caps",
        });
    }

    let spec: CompileProfileSpec = value.try_into()?;
    validate_compile_profile_spec_v2(&spec)?;
    emit_profile_spec_v2_loaded(&spec);
    Ok(spec)
}

pub fn canonical_compile_profile_specs()
-> Result<[CompileProfileSpec; 4], CompileProfileSpecLoadError> {
    Ok([
        load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(TRACE_COMPILE_PROFILE_TOML)?,
        load_compile_profile_spec(RECOVERY_COMPILE_PROFILE_TOML)?,
    ])
}

pub fn resolve_repair_policy_from_profile_spec(
    profile: &CompileProfileSpec,
) -> Result<RepairPolicy, InitialKnobsResolveError> {
    Ok(RepairPolicy::for_profile(repair_policy_profile(
        profile.id.clone(),
    )?))
}

pub fn resolve_initial_knobs_from_profile_spec(
    profile: &CompileProfileSpec,
) -> Result<CompileKnobs, InitialKnobsResolveError> {
    let profile_kind = repair_policy_profile(profile.id.clone())?;
    let global = CompileKnobValues {
        placement: require_default(profile, profile.knob_defaults.placement, "placement")?,
        observation: require_default(profile, profile.knob_defaults.observation, "observation")?,
        range: require_default(profile, profile.knob_defaults.range, "range")?,
        storage: require_default(profile, profile.knob_defaults.storage, "storage")?,
        sram: require_default(profile, profile.knob_defaults.sram, "sram")?,
        rom_window: require_default(profile, profile.knob_defaults.rom_window, "rom_window")?,
        overlay: require_default(profile, profile.knob_defaults.overlay, "overlay")?,
        schedule: require_default(profile, profile.knob_defaults.schedule, "schedule")?,
    };
    let bounds = CompileKnobBounds {
        placement: require_bound(profile, profile.knob_bounds.placement, "placement")?,
        observation: require_bound(profile, profile.knob_bounds.observation, "observation")?,
        range: require_bound(profile, profile.knob_bounds.range, "range")?,
        storage: require_bound(profile, profile.knob_bounds.storage, "storage")?,
        sram: require_bound(profile, profile.knob_bounds.sram, "sram")?,
        rom_window: require_bound(profile, profile.knob_bounds.rom_window, "rom_window")?,
        overlay: require_bound(profile, profile.knob_bounds.overlay, "overlay")?,
        schedule: require_bound(profile, profile.knob_bounds.schedule, "schedule")?,
    };

    Ok(CompileKnobs {
        global,
        bounds,
        locks: f_b16_profile_lock_set(profile_kind),
        overrides: CompileKnobOverrides::default(),
        provenance: profile_default_provenance(profile),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcePressureThresholdResolution {
    pub thresholds: ResourcePressureThresholds,
    pub provenance: Vec<CompileKnobProvenanceEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourcePressureThresholds {
    pub wram_hot: PressureLimit<ByteBudget>,
    pub hram_hot: PressureLimit<ByteBudget>,
    pub bank0_rom: PressureLimit<ByteBudget>,
    pub switchable_rom_window: PressureLimit<ByteBudget>,
    pub sram_window: PressureLimit<ByteBudget>,
    pub slice_cycles: PressureLimit<CycleBudget>,
    pub interrupt_latency: PressureLimit<CycleBudget>,
    pub trace_bytes_per_frame: PressureLimit<u16>,
    pub persist_bytes_per_frame: PressureLimit<u16>,
    pub overlay_installs_per_frame: PressureLimit<u8>,
    pub bank_switches_per_token: PressureLimit<u16>,
    pub sram_page_switches_per_token: PressureLimit<u16>,
}

impl Default for ResourcePressureThresholds {
    fn default() -> Self {
        Self {
            wram_hot: PressureLimit {
                soft: 6 * 1024,
                hard: 8 * 1024,
            },
            hram_hot: PressureLimit {
                soft: 96,
                hard: 127,
            },
            bank0_rom: PressureLimit {
                soft: 14 * 1024,
                hard: 16 * 1024,
            },
            switchable_rom_window: PressureLimit {
                soft: 14 * 1024,
                hard: 16 * 1024,
            },
            sram_window: PressureLimit {
                soft: 6 * 1024,
                hard: 8 * 1024,
            },
            slice_cycles: PressureLimit {
                soft: 4_000,
                hard: 8_000,
            },
            interrupt_latency: PressureLimit {
                soft: 512,
                hard: 1_024,
            },
            trace_bytes_per_frame: PressureLimit {
                soft: 512,
                hard: 1_024,
            },
            persist_bytes_per_frame: PressureLimit {
                soft: 512,
                hard: 1_024,
            },
            overlay_installs_per_frame: PressureLimit { soft: 1, hard: 4 },
            bank_switches_per_token: PressureLimit { soft: 2, hard: 8 },
            sram_page_switches_per_token: PressureLimit { soft: 1, hard: 4 },
        }
    }
}

impl ResourcePressureThresholds {
    #[must_use]
    pub fn all_le(self, other: Self) -> bool {
        pressure_limit_le(&self.wram_hot, &other.wram_hot)
            && pressure_limit_le(&self.hram_hot, &other.hram_hot)
            && pressure_limit_le(&self.bank0_rom, &other.bank0_rom)
            && pressure_limit_le(&self.switchable_rom_window, &other.switchable_rom_window)
            && pressure_limit_le(&self.sram_window, &other.sram_window)
            && pressure_limit_le(&self.slice_cycles, &other.slice_cycles)
            && pressure_limit_le(&self.interrupt_latency, &other.interrupt_latency)
            && pressure_limit_le(&self.trace_bytes_per_frame, &other.trace_bytes_per_frame)
            && pressure_limit_le(
                &self.persist_bytes_per_frame,
                &other.persist_bytes_per_frame,
            )
            && pressure_limit_le(
                &self.overlay_installs_per_frame,
                &other.overlay_installs_per_frame,
            )
            && pressure_limit_le(
                &self.bank_switches_per_token,
                &other.bank_switches_per_token,
            )
            && pressure_limit_le(
                &self.sram_page_switches_per_token,
                &other.sram_page_switches_per_token,
            )
    }

    #[must_use]
    pub fn all_ge(self, other: Self) -> bool {
        other.all_le(self)
    }
}

fn pressure_limit_le<T>(left: &PressureLimit<T>, right: &PressureLimit<T>) -> bool
where
    T: PartialOrd,
{
    left.soft <= right.soft && left.hard <= right.hard
}

pub fn resolve_resource_pressure_thresholds(
    profile: &CompileProfileSpec,
    runtime_chrome_budget: Option<&RuntimeChromeBudget>,
    objective: &CompileObjective,
    trace_budget: &TraceBudget,
) -> Result<ResourcePressureThresholdResolution, InitialKnobsResolveError> {
    let schedule = require_default(profile, profile.knob_defaults.schedule, "schedule")?;
    let pressure = schedule.resource_pressure;
    let thresholds = ResourcePressureThresholds {
        wram_hot: byte_limit(
            runtime_chrome_budget
                .map(wram_hot_hard_bytes)
                .unwrap_or_else(|| {
                    profile_default_byte_hard(pressure, ProfilePressureField::WramHot)
                }),
            pressure,
        ),
        hram_hot: byte_limit(
            runtime_chrome_budget
                .map(|budget| u64::from(budget.memory_caps.hram_usable_bytes))
                .unwrap_or_else(|| {
                    profile_default_byte_hard(pressure, ProfilePressureField::HramHot)
                }),
            pressure,
        ),
        bank0_rom: byte_limit(
            runtime_chrome_budget
                .map(bank0_rom_hard_bytes)
                .unwrap_or_else(|| {
                    profile_default_byte_hard(pressure, ProfilePressureField::Bank0Rom)
                }),
            pressure,
        ),
        switchable_rom_window: byte_limit(
            runtime_chrome_budget
                .map(switchable_rom_window_hard_bytes)
                .unwrap_or_else(|| {
                    profile_default_byte_hard(pressure, ProfilePressureField::SwitchableRomWindow)
                }),
            pressure,
        ),
        sram_window: byte_limit(
            runtime_chrome_budget
                .map(sram_window_hard_bytes)
                .unwrap_or_else(|| {
                    profile_default_byte_hard(pressure, ProfilePressureField::SramWindow)
                }),
            pressure,
        ),
        slice_cycles: cycle_limit(
            objective
                .max_cycles_per_token
                .map(u64::from)
                .unwrap_or_else(|| {
                    profile_default_cycle_hard(pressure, ProfilePressureField::SliceCycles)
                }),
            pressure,
        ),
        interrupt_latency: cycle_limit(
            objective
                .service
                .as_ref()
                .and_then(|service| {
                    service
                        .max_resume_latency_cycles_p95
                        .or(service.max_checkpoint_gap_cycles_p95)
                        .or(service.max_first_token_cycles_p95)
                })
                .map(u64::from)
                .unwrap_or_else(|| {
                    profile_default_cycle_hard(pressure, ProfilePressureField::InterruptLatency)
                }),
            pressure,
        ),
        trace_bytes_per_frame: u16_limit(trace_budget.max_bytes_per_frame, pressure),
        persist_bytes_per_frame: u16_limit(
            runtime_chrome_budget
                .map(|budget| saturating_u16(budget.sram_reserved))
                .unwrap_or_else(|| {
                    profile_default_u16_hard(pressure, ProfilePressureField::PersistBytesPerFrame)
                }),
            pressure,
        ),
        overlay_installs_per_frame: u8_limit(
            profile_default_u8_hard(pressure, ProfilePressureField::OverlayInstallsPerFrame),
            pressure,
        ),
        bank_switches_per_token: u16_limit(
            objective.max_bank_switches_per_token.unwrap_or_else(|| {
                profile_default_u16_hard(pressure, ProfilePressureField::BankSwitchesPerToken)
            }),
            pressure,
        ),
        sram_page_switches_per_token: u16_limit(
            objective
                .max_sram_page_switches_per_token
                .unwrap_or_else(|| {
                    profile_default_u16_hard(
                        pressure,
                        ProfilePressureField::SramPageSwitchesPerToken,
                    )
                }),
            pressure,
        ),
    };

    Ok(ResourcePressureThresholdResolution {
        thresholds,
        provenance: resource_pressure_provenance(
            profile,
            runtime_chrome_budget,
            objective,
            trace_budget,
        ),
    })
}

#[derive(Debug, Clone, Copy)]
enum ProfilePressureField {
    WramHot,
    HramHot,
    Bank0Rom,
    SwitchableRomWindow,
    SramWindow,
    SliceCycles,
    InterruptLatency,
    PersistBytesPerFrame,
    OverlayInstallsPerFrame,
    BankSwitchesPerToken,
    SramPageSwitchesPerToken,
}

fn wram_hot_hard_bytes(budget: &RuntimeChromeBudget) -> u64 {
    u64::from(
        budget
            .memory_caps
            .wram_usable_bytes
            .saturating_sub(u32::from(budget.wram_reserved)),
    )
}

fn sram_window_hard_bytes(budget: &RuntimeChromeBudget) -> u64 {
    u64::from(
        budget
            .memory_caps
            .sram_usable_bytes
            .saturating_sub(budget.sram_reserved),
    )
}

fn bank0_rom_hard_bytes(budget: &RuntimeChromeBudget) -> u64 {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class == crate::budget::BudgetSlotClass::Bank0Free)
        .map(slot_hard_bytes)
        .sum()
}

fn switchable_rom_window_hard_bytes(budget: &RuntimeChromeBudget) -> u64 {
    budget
        .rom_slots
        .iter()
        .filter(|slot| slot.class != crate::budget::BudgetSlotClass::Bank0Free)
        .map(slot_hard_bytes)
        .sum()
}

fn slot_hard_bytes(slot: &crate::budget::RomBudgetSlot) -> u64 {
    u64::from(
        slot.usable_bytes
            .saturating_sub(u32::from(slot.reserved_slack)),
    )
}

fn byte_limit(hard: u64, pressure: ScheduleResourcePressure) -> PressureLimit<ByteBudget> {
    PressureLimit {
        soft: soften_u64(hard, pressure),
        hard,
    }
}

fn cycle_limit(hard: u64, pressure: ScheduleResourcePressure) -> PressureLimit<CycleBudget> {
    byte_limit(hard, pressure)
}

fn u16_limit(hard: u16, pressure: ScheduleResourcePressure) -> PressureLimit<u16> {
    PressureLimit {
        soft: soften_u16(hard, pressure),
        hard,
    }
}

fn u8_limit(hard: u8, pressure: ScheduleResourcePressure) -> PressureLimit<u8> {
    PressureLimit {
        soft: soften_u8(hard, pressure),
        hard,
    }
}

fn soften_u64(hard: u64, pressure: ScheduleResourcePressure) -> u64 {
    let numerator = match pressure {
        ScheduleResourcePressure::Conservative => 4,
        ScheduleResourcePressure::Balanced => 9,
        ScheduleResourcePressure::FitFirst => 19,
    };
    let denominator = match pressure {
        ScheduleResourcePressure::Conservative => 5,
        ScheduleResourcePressure::Balanced => 10,
        ScheduleResourcePressure::FitFirst => 20,
    };
    hard.saturating_mul(numerator) / denominator
}

fn soften_u16(hard: u16, pressure: ScheduleResourcePressure) -> u16 {
    soften_u64(u64::from(hard), pressure)
        .try_into()
        .unwrap_or(u16::MAX)
}

fn soften_u8(hard: u8, pressure: ScheduleResourcePressure) -> u8 {
    soften_u64(u64::from(hard), pressure)
        .try_into()
        .unwrap_or(u8::MAX)
}

fn saturating_u16(value: u32) -> u16 {
    value.try_into().unwrap_or(u16::MAX)
}

fn profile_default_byte_hard(
    pressure: ScheduleResourcePressure,
    field: ProfilePressureField,
) -> u64 {
    let base = match field {
        ProfilePressureField::WramHot => 4096,
        ProfilePressureField::HramHot => 96,
        ProfilePressureField::Bank0Rom => 4096,
        ProfilePressureField::SwitchableRomWindow => 16 * 1024,
        ProfilePressureField::SramWindow => 8 * 1024,
        _ => 1024,
    };
    pressure_scaled_u64(base, pressure)
}

fn profile_default_cycle_hard(
    pressure: ScheduleResourcePressure,
    field: ProfilePressureField,
) -> u64 {
    let base = match field {
        ProfilePressureField::SliceCycles => 8192,
        ProfilePressureField::InterruptLatency => 2048,
        _ => 1024,
    };
    pressure_scaled_u64(base, pressure)
}

fn profile_default_u16_hard(
    pressure: ScheduleResourcePressure,
    field: ProfilePressureField,
) -> u16 {
    let base = match field {
        ProfilePressureField::PersistBytesPerFrame => 512,
        ProfilePressureField::BankSwitchesPerToken => 8,
        ProfilePressureField::SramPageSwitchesPerToken => 4,
        _ => 1,
    };
    pressure_scaled_u64(base, pressure)
        .try_into()
        .unwrap_or(u16::MAX)
}

fn profile_default_u8_hard(pressure: ScheduleResourcePressure, field: ProfilePressureField) -> u8 {
    let base = match field {
        ProfilePressureField::OverlayInstallsPerFrame => 1,
        _ => 1,
    };
    pressure_scaled_u64(base, pressure)
        .try_into()
        .unwrap_or(u8::MAX)
}

fn pressure_scaled_u64(base: u64, pressure: ScheduleResourcePressure) -> u64 {
    match pressure {
        ScheduleResourcePressure::Conservative => base,
        ScheduleResourcePressure::Balanced => base.saturating_mul(2),
        ScheduleResourcePressure::FitFirst => base.saturating_mul(4),
    }
}

fn resource_pressure_provenance(
    profile: &CompileProfileSpec,
    runtime_chrome_budget: Option<&RuntimeChromeBudget>,
    objective: &CompileObjective,
    trace_budget: &TraceBudget,
) -> Vec<CompileKnobProvenanceEntry> {
    let runtime = runtime_chrome_budget.map(runtime_chrome_provenance);
    let profile_default = profile_pressure_provenance(profile);
    let scheduler = scheduler_pressure_provenance(profile);
    let objective = objective_pressure_provenance(objective);
    let trace = trace_budget_provenance(trace_budget);

    [
        (
            "thresholds.wram_hot",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.hram_hot",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.bank0_rom",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.switchable_rom_window",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.sram_window",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.slice_cycles",
            vec![
                profile_default.clone(),
                scheduler.clone(),
                objective.clone(),
            ],
        ),
        (
            "thresholds.interrupt_latency",
            vec![
                profile_default.clone(),
                scheduler.clone(),
                objective.clone(),
            ],
        ),
        (
            "thresholds.trace_bytes_per_frame",
            vec![profile_default.clone(), trace],
        ),
        (
            "thresholds.persist_bytes_per_frame",
            vec![
                profile_default.clone(),
                optional_or_profile(runtime.clone(), &profile_default),
            ],
        ),
        (
            "thresholds.overlay_installs_per_frame",
            vec![profile_default.clone(), scheduler.clone()],
        ),
        (
            "thresholds.bank_switches_per_token",
            vec![
                profile_default.clone(),
                scheduler.clone(),
                objective.clone(),
            ],
        ),
        (
            "thresholds.sram_page_switches_per_token",
            vec![profile_default, scheduler, objective],
        ),
    ]
    .into_iter()
    .map(|(field, chain)| CompileKnobProvenanceEntry {
        path: CompileKnobPath {
            knob: CompileKnobId::ScheduleResourcePressure,
            selector: None,
            field: Some(FieldPath::from(field)),
        },
        chain,
    })
    .collect()
}

fn optional_or_profile(
    source: Option<ConstraintProvenance>,
    profile: &ConstraintProvenance,
) -> ConstraintProvenance {
    source.unwrap_or_else(|| profile.clone())
}

fn profile_pressure_provenance(profile: &CompileProfileSpec) -> ConstraintProvenance {
    ConstraintProvenance {
        source: PolicySource::ProfileDefault,
        operation: ConstraintOperation::SeedDefault,
        evidence: vec![EvidenceRef {
            kind: "CompileProfileSpec".to_owned(),
            reference: format!("{}.schedule.resource_pressure", profile.id.as_str()),
            hash: Some(profile.defaults_hash),
        }],
    }
}

fn scheduler_pressure_provenance(profile: &CompileProfileSpec) -> ConstraintProvenance {
    ConstraintProvenance {
        source: PolicySource::ProfileDefault,
        operation: ConstraintOperation::ApplyPreference,
        evidence: vec![EvidenceRef {
            kind: "ScheduleResourcePressure".to_owned(),
            reference: profile.id.as_str().to_owned(),
            hash: Some(profile.defaults_hash),
        }],
    }
}

fn runtime_chrome_provenance(budget: &RuntimeChromeBudget) -> ConstraintProvenance {
    ConstraintProvenance {
        source: PolicySource::TargetDefault,
        operation: ConstraintOperation::ApplyHardConstraint,
        evidence: vec![EvidenceRef {
            kind: "RuntimeChromeBudget".to_owned(),
            reference: format!("{}:{}", budget.target.as_str(), budget.profile.as_str()),
            hash: Some(budget.runtime_nucleus_hash),
        }],
    }
}

fn objective_pressure_provenance(objective: &CompileObjective) -> ConstraintProvenance {
    ConstraintProvenance {
        source: PolicySource::CompileRequestOverride,
        operation: ConstraintOperation::ApplyHardConstraint,
        evidence: vec![EvidenceRef {
            kind: "CompileObjective".to_owned(),
            reference: format!(
                "cycles={:?};bank_switches={:?};sram_page_switches={:?}",
                objective.max_cycles_per_token,
                objective.max_bank_switches_per_token,
                objective.max_sram_page_switches_per_token
            ),
            hash: None,
        }],
    }
}

fn trace_budget_provenance(trace_budget: &TraceBudget) -> ConstraintProvenance {
    ConstraintProvenance {
        source: PolicySource::ProfileDefault,
        operation: ConstraintOperation::ApplyHardConstraint,
        evidence: vec![EvidenceRef {
            kind: "TraceBudget".to_owned(),
            reference: format!(
                "events_per_slice={};bytes_per_frame={}",
                trace_budget.max_events_per_slice, trace_budget.max_bytes_per_frame
            ),
            hash: None,
        }],
    }
}

fn repair_policy_profile(
    profile: CompileProfileId,
) -> Result<RepairPolicyProfile, InitialKnobsResolveError> {
    match profile.as_str() {
        BRINGUP_COMPILE_PROFILE_ID => Ok(RepairPolicyProfile::Bringup),
        DEFAULT_COMPILE_PROFILE_ID => Ok(RepairPolicyProfile::Default),
        TRACE_COMPILE_PROFILE_ID => Ok(RepairPolicyProfile::TraceInvariant),
        RECOVERY_COMPILE_PROFILE_ID => Ok(RepairPolicyProfile::Recovery),
        _ => Err(InitialKnobsResolveError::UnknownProfile { profile }),
    }
}

fn require_default<T: Copy>(
    profile: &CompileProfileSpec,
    value: Option<T>,
    field: &'static str,
) -> Result<T, InitialKnobsResolveError> {
    value.ok_or_else(|| InitialKnobsResolveError::MissingDefault {
        profile: profile.id.clone(),
        field,
    })
}

fn require_bound<T: Copy>(
    profile: &CompileProfileSpec,
    value: Option<T>,
    field: &'static str,
) -> Result<T, InitialKnobsResolveError> {
    value.ok_or_else(|| InitialKnobsResolveError::MissingBound {
        profile: profile.id.clone(),
        field,
    })
}

fn profile_default_provenance(profile: &CompileProfileSpec) -> Vec<CompileKnobProvenanceEntry> {
    [
        CompileKnobId::Placement,
        CompileKnobId::Observation,
        CompileKnobId::Range,
        CompileKnobId::Storage,
        CompileKnobId::Sram,
        CompileKnobId::RomWindow,
        CompileKnobId::Overlay,
        CompileKnobId::Schedule,
    ]
    .into_iter()
    .map(|knob| CompileKnobProvenanceEntry {
        path: CompileKnobPath {
            knob,
            selector: None,
            field: Some(FieldPath::from(profile_default_field(knob))),
        },
        chain: vec![ConstraintProvenance {
            source: PolicySource::ProfileDefault,
            operation: ConstraintOperation::SeedDefault,
            evidence: vec![EvidenceRef {
                kind: "CompileProfileSpec".to_owned(),
                reference: profile.id.as_str().to_owned(),
                hash: Some(profile.defaults_hash),
            }],
        }],
    })
    .collect()
}

const fn profile_default_field(knob: CompileKnobId) -> &'static str {
    match knob {
        CompileKnobId::Placement => "profile_default.placement",
        CompileKnobId::Observation => "profile_default.observation",
        CompileKnobId::Range => "profile_default.range",
        CompileKnobId::Storage => "profile_default.storage",
        CompileKnobId::Sram => "profile_default.sram",
        CompileKnobId::RomWindow => "profile_default.rom_window",
        CompileKnobId::Overlay => "profile_default.overlay",
        CompileKnobId::Schedule => "profile_default.schedule",
        _ => "profile_default",
    }
}

fn validate_compile_profile_spec_v2(
    spec: &CompileProfileSpec,
) -> Result<(), CompileProfileSpecLoadError> {
    if spec.range_caps.profile_chunk_max == 0 {
        emit_profile_spec_v2_invariant_failure(
            spec,
            "profile_chunk_max > 0",
            "range_caps.profile_chunk_max",
            &spec.range_caps.profile_chunk_max.to_string(),
        );
        return Err(CompileProfileSpecLoadError::InvalidInvariant {
            invariant: "profile_chunk_max > 0",
            field: "range_caps.profile_chunk_max",
            value: spec.range_caps.profile_chunk_max.to_string(),
        });
    }
    if spec.range_caps.profile_tile_max == 0 {
        emit_profile_spec_v2_invariant_failure(
            spec,
            "profile_tile_max > 0",
            "range_caps.profile_tile_max",
            &spec.range_caps.profile_tile_max.to_string(),
        );
        return Err(CompileProfileSpecLoadError::InvalidInvariant {
            invariant: "profile_tile_max > 0",
            field: "range_caps.profile_tile_max",
            value: spec.range_caps.profile_tile_max.to_string(),
        });
    }
    if spec.range_caps.profile_tile_min == 0 {
        emit_profile_spec_v2_invariant_failure(
            spec,
            "profile_tile_min > 0",
            "range_caps.profile_tile_min",
            &spec.range_caps.profile_tile_min.to_string(),
        );
        return Err(CompileProfileSpecLoadError::InvalidInvariant {
            invariant: "profile_tile_min > 0",
            field: "range_caps.profile_tile_min",
            value: spec.range_caps.profile_tile_min.to_string(),
        });
    }
    if spec.range_caps.profile_tile_min > spec.range_caps.profile_tile_max {
        emit_profile_spec_v2_invariant_failure(
            spec,
            "profile_tile_min <= profile_tile_max",
            "range_caps.profile_tile_min",
            &spec.range_caps.profile_tile_min.to_string(),
        );
        return Err(CompileProfileSpecLoadError::InvalidInvariant {
            invariant: "profile_tile_min <= profile_tile_max",
            field: "range_caps.profile_tile_min",
            value: spec.range_caps.profile_tile_min.to_string(),
        });
    }
    if let RenormStrategyPolicy::DynamicMargin { margin_q16_16 } = spec.range_caps.renorm_strategy
        && margin_q16_16 >= 0x1_0000
    {
        emit_profile_spec_v2_invariant_failure(
            spec,
            "DynamicMargin.margin_q16_16 < 0x1_0000",
            "range_caps.renorm_strategy.margin_q16_16",
            &margin_q16_16.to_string(),
        );
        return Err(CompileProfileSpecLoadError::InvalidInvariant {
            invariant: "DynamicMargin.margin_q16_16 < 0x1_0000",
            field: "range_caps.renorm_strategy.margin_q16_16",
            value: margin_q16_16.to_string(),
        });
    }
    Ok(())
}

fn schema_version_literal(value: &toml::Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn unsupported_schema_version_error(
    actual: impl Into<String>,
    reason: &'static str,
) -> CompileProfileSpecLoadError {
    let actual = actual.into();
    if actual == "1.0.0" {
        emit_profile_spec_v1_rejected(&actual, reason);
    }
    CompileProfileSpecLoadError::UnsupportedSchemaVersion {
        code: COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE,
        actual,
        expected: COMPILE_PROFILE_SPEC_VERSION,
    }
}

fn compile_profile_component_hash<T: Serialize>(
    type_name: &'static str,
    value: &T,
) -> Result<Hash256, serde_json::Error> {
    domain_hash("gbf-policy", type_name, COMPILE_PROFILE_SPEC_VERSION, value)
}

fn emit_profile_spec_v2_loaded(spec: &CompileProfileSpec) {
    let range_caps_hash = compile_profile_component_hash("RangeCapsSpec", &spec.range_caps)
        .expect("range caps telemetry hash serializes")
        .to_hex();
    let observation_caps_hash =
        compile_profile_component_hash("ObservationProfileCaps", &spec.observation_caps)
            .expect("observation caps telemetry hash serializes")
            .to_hex();

    tracing::info!(
        event = PROFILE_SPEC_V2_LOADED_EVENT,
        profile_id = spec.id.as_str(),
        schema_version = spec.schema_version.as_str(),
        range_caps_hash = range_caps_hash.as_str(),
        observation_caps_hash = observation_caps_hash.as_str(),
    );
}

fn emit_profile_spec_v1_rejected(actual_version: &str, reason: &'static str) {
    tracing::info!(
        event = PROFILE_SPEC_V1_REJECTED_EVENT,
        actual_version,
        fixture_path = PROFILE_SPEC_INLINE_FIXTURE_PATH,
        reason,
    );
}

fn emit_profile_spec_v2_invariant_failure(
    spec: &CompileProfileSpec,
    invariant: &'static str,
    field: &'static str,
    value: &str,
) {
    tracing::info!(
        event = PROFILE_SPEC_V2_INVARIANT_FAILURE_EVENT,
        profile_id = spec.id.as_str(),
        schema_version = spec.schema_version.as_str(),
        invariant,
        field,
        value,
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveConstraints {
    pub target_caps: CompileKnobBounds,
    pub required_features: BTreeSet<CompilerFeature>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub runtime_chrome_budget: Option<RuntimeChromeBudget>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobs {
    pub global: CompileKnobValues,
    pub bounds: CompileKnobBounds,
    pub locks: KnobLockSet,
    pub overrides: CompileKnobOverrides,
    pub provenance: Vec<CompileKnobProvenanceEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobValues {
    pub placement: PlacementKnob,
    pub observation: ObservationKnob,
    pub range: RangeKnob,
    pub storage: StorageKnob,
    pub sram: SramKnob,
    pub rom_window: RomWindowKnob,
    pub overlay: OverlayKnob,
    pub schedule: ScheduleKnob,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPartialValues {
    pub placement: Option<PlacementKnob>,
    pub observation: Option<ObservationKnob>,
    pub range: Option<RangeKnob>,
    pub storage: Option<StorageKnob>,
    pub sram: Option<SramKnob>,
    pub rom_window: Option<RomWindowKnob>,
    pub overlay: Option<OverlayKnob>,
    pub schedule: Option<ScheduleKnob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobBounds {
    pub placement: PlacementKnobBounds,
    pub observation: ObservationKnobBounds,
    pub range: RangeKnobBounds,
    pub storage: StorageKnobBounds,
    pub sram: SramKnobBounds,
    pub rom_window: RomWindowKnobBounds,
    pub overlay: OverlayKnobBounds,
    pub schedule: ScheduleKnobBounds,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPartialBounds {
    pub placement: Option<PlacementKnobBounds>,
    pub observation: Option<ObservationKnobBounds>,
    pub range: Option<RangeKnobBounds>,
    pub storage: Option<StorageKnobBounds>,
    pub sram: Option<SramKnobBounds>,
    pub rom_window: Option<RomWindowKnobBounds>,
    pub overlay: Option<OverlayKnobBounds>,
    pub schedule: Option<ScheduleKnobBounds>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobOverrides {
    #[serde(default)]
    pub values: CompileKnobPartialValues,
    #[serde(default)]
    pub bounds: CompileKnobPartialBounds,
    /// Optional operational probes disabled by trace demotion.
    ///
    /// Semantic observations are never listed here; the loop only records
    /// optional probe removals.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub disabled_optional_probes: BTreeSet<TraceProbeId>,
    /// Per-kernel residency requirements inserted by repair.
    ///
    /// Monotone repair may insert a selector or move an existing selector to a
    /// later residency rank. It may not delete entries or move them backward.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "map_as_pairs"
    )]
    pub forced_kernel_residency: BTreeMap<KernelSelector, KernelResidency>,
    /// Pure values forced from materialize to recompute. Effectful values are
    /// rejected by the loop before insertion.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub forced_recompute: BTreeSet<ValueSelector>,
    /// Site-specific range-plan ceilings.
    ///
    /// Monotone repair may insert a selector or raise its ceiling only.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "map_as_pairs"
    )]
    pub reduction_ceiling_overrides: BTreeMap<ReductionSelector, ReductionPlanCeiling>,
    /// Legal tile classes remaining after candidate narrowing.
    ///
    /// Monotone repair is subset-removal for existing selectors.
    #[serde(
        default,
        skip_serializing_if = "BTreeMap::is_empty",
        with = "map_as_pairs"
    )]
    pub tile_class_overrides: BTreeMap<TileSelector, BTreeSet<TileCandidateClass>>,
}

mod map_as_pairs {
    use super::*;
    use serde::de::{SeqAccess, Visitor};
    use serde::{Deserializer, Serializer};
    use std::fmt;
    use std::marker::PhantomData;

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, serializer: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize,
        V: Serialize,
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(map.len()))?;
        for entry in map {
            seq.serialize_element(&entry)?;
        }
        seq.end()
    }

    pub fn deserialize<'de, K, V, D>(deserializer: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        deserializer.deserialize_seq(MapVisitor {
            marker: PhantomData,
        })
    }

    struct MapVisitor<K, V> {
        marker: PhantomData<(K, V)>,
    }

    impl<'de, K, V> Visitor<'de> for MapVisitor<K, V>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
    {
        type Value = BTreeMap<K, V>;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("a sequence of selector/value pairs")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
        where
            A: SeqAccess<'de>,
        {
            let mut map = BTreeMap::new();
            while let Some((key, value)) = seq.next_element()? {
                map.insert(key, value);
            }
            Ok(map)
        }
    }
}

impl CompileKnobPartialValues {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.placement.is_none()
            && self.observation.is_none()
            && self.range.is_none()
            && self.storage.is_none()
            && self.sram.is_none()
            && self.rom_window.is_none()
            && self.overlay.is_none()
            && self.schedule.is_none()
    }
}

impl CompileKnobPartialBounds {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.placement.is_none()
            && self.observation.is_none()
            && self.range.is_none()
            && self.storage.is_none()
            && self.sram.is_none()
            && self.rom_window.is_none()
            && self.overlay.is_none()
            && self.schedule.is_none()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnobLockSet {
    pub locked: BTreeSet<CompileKnobId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CompileKnobId {
    Placement,
    Observation,
    Range,
    Storage,
    Sram,
    RomWindow,
    Overlay,
    Schedule,
    PlacementProfile,
    ObservationTraceDemotion,
    ObservationProbeSelection,
    RangeReductionCeiling,
    StorageRecomputePromotion,
    StorageMaterializationOverrides,
    SramPageAggression,
    SramSpillPolicy,
    RomKernelResidencyBias,
    RomKernelDuplicationBias,
    RomKernelResidencyOverrides,
    OverlayPromotion,
    ScheduleTileSearch,
    ScheduleSliceCoarsening,
    ScheduleResourcePressure,
    StageIterationCeilings,
}

impl CompileKnobId {
    #[must_use]
    pub const fn top_level(self) -> Self {
        match self {
            Self::Placement | Self::PlacementProfile => Self::Placement,
            Self::Observation
            | Self::ObservationTraceDemotion
            | Self::ObservationProbeSelection => Self::Observation,
            Self::Range | Self::RangeReductionCeiling => Self::Range,
            Self::Storage
            | Self::StorageRecomputePromotion
            | Self::StorageMaterializationOverrides => Self::Storage,
            Self::Sram | Self::SramPageAggression | Self::SramSpillPolicy => Self::Sram,
            Self::RomWindow
            | Self::RomKernelResidencyBias
            | Self::RomKernelDuplicationBias
            | Self::RomKernelResidencyOverrides => Self::RomWindow,
            Self::Overlay | Self::OverlayPromotion => Self::Overlay,
            Self::Schedule
            | Self::ScheduleTileSearch
            | Self::ScheduleSliceCoarsening
            | Self::ScheduleResourcePressure
            | Self::StageIterationCeilings => Self::Schedule,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobPath {
    pub knob: CompileKnobId,
    pub selector: Option<SelectorPath>,
    pub field: Option<FieldPath>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileKnobProvenanceEntry {
    pub path: CompileKnobPath,
    pub chain: Vec<ConstraintProvenance>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintProvenance {
    pub source: PolicySource,
    pub operation: ConstraintOperation,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PolicySource {
    TargetDefault,
    ProfileDefault,
    CompileRequestOverride,
    HintBundle,
    Calibration,
    RepairProposal { id: RepairProposalId },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
    AppliedRepairProposal { id: RepairProposalId },
    AuthorizedRelaxation { reason: RepairReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KernelSpecId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LayerId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExpertId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SectionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ValueId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AliasClassId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum KernelSelector {
    KernelSpec { id: KernelSpecId },
    LayerExpert { layer: LayerId, expert: ExpertId },
    Section { id: SectionId },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ValueSelector {
    Value { id: ValueId },
    AliasClass { id: AliasClassId },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionSelector {
    Site { id: ReductionSiteId },
    Layer { id: LayerId },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TileSelector {
    Kernel { id: KernelSpecId },
    Layer { id: LayerId },
    SliceClass { class: SliceClass },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SliceClass {
    Micro,
    Frame,
    TokenBoundary,
    TraceHeavy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum KernelResidency {
    ProfileDefault = 0,
    CoResident = 1,
    Bank0Streaming = 2,
    WramOverlay = 3,
    FitFirst = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TileCandidateClass {
    SmallWorkingSet,
    Balanced,
    SwitchAmortized,
}

pub type ByteBudget = u64;
pub type CycleBudget = u64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PressureLimit<T> {
    pub soft: T,
    pub hard: T,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConstraintDelta {
    pub changes: Vec<KnobDelta>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecomputePurityFacts {
    /// Values proven recomputable by the current IR/predicate surface.
    pub pure_values: BTreeSet<ValueSelector>,
    /// Values known to write effects or otherwise fail the current recompute
    /// purity contract.
    pub effectful_values: BTreeSet<ValueSelector>,
}

impl RecomputePurityFacts {
    #[must_use]
    pub fn allows(&self, value: &ValueSelector) -> bool {
        self.pure_values.contains(value) && !self.effectful_values.contains(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum KnobDelta {
    AdvancePlacementProfile {
        to: PlacementProfile,
    },
    SetTraceDemotion {
        to: TraceDemotionLevel,
    },
    DisableOptionalProbes {
        probes: BTreeSet<TraceProbeId>,
    },
    RaiseReductionCeiling {
        selector: Option<ReductionSelector>,
        to: ReductionPlanCeiling,
    },
    PromoteRecomputeLevel {
        to: StorageMaterialization,
    },
    ForceRecompute {
        values: BTreeSet<ValueSelector>,
    },
    AdvanceSramPageAggression {
        to: SramPageAggression,
    },
    AdvanceSramSpillPolicy {
        to: SramSpillPolicy,
    },
    AdvanceKernelResidencyBias {
        to: RomKernelResidencyBias,
    },
    AdvanceKernelDuplicationBias {
        to: RomKernelDuplicationBias,
    },
    ForceKernelResidency {
        selector: KernelSelector,
        to: KernelResidency,
    },
    PromoteOverlay {
        to: OverlayPromotion,
    },
    NarrowTileClasses {
        selector: TileSelector,
        remaining: BTreeSet<TileCandidateClass>,
    },
    SetSliceCoarsening {
        to: ScheduleSliceCoarsening,
    },
    UpdatePressureThreshold {
        update: ResourcePressureUpdate,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ResourcePressureUpdate {
    WramHot { limit: PressureLimit<ByteBudget> },
    HramHot { limit: PressureLimit<ByteBudget> },
    Bank0Rom { limit: PressureLimit<ByteBudget> },
    SwitchableRomWindow { limit: PressureLimit<ByteBudget> },
    SramWindow { limit: PressureLimit<ByteBudget> },
    SliceCycles { limit: PressureLimit<CycleBudget> },
    InterruptLatency { limit: PressureLimit<CycleBudget> },
    TraceBytesPerFrame { limit: PressureLimit<u16> },
    PersistBytesPerFrame { limit: PressureLimit<u16> },
    OverlayInstallsPerFrame { limit: PressureLimit<u8> },
    BankSwitchesPerToken { limit: PressureLimit<u16> },
    SramPageSwitchesPerToken { limit: PressureLimit<u16> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum DeltaRejection {
    AcceptedRefinementBudgetExhausted {
        max_refinement_iters: u8,
    },
    KnobLocked {
        knob: CompileKnobId,
    },
    PolicyToggleDisabled {
        knob: CompileKnobId,
        toggle: String,
    },
    BeyondBounds {
        knob: CompileKnobId,
        attempted: String,
        max: String,
    },
    NotMonotone {
        knob: CompileKnobId,
        current: String,
        attempted: String,
    },
    InvariantObservabilityViolation {
        knob: CompileKnobId,
    },
    EffectfulRecompute {
        value: ValueSelector,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyProvenance {
    /// Source-schema hashes for inputs that always participate in policy resolution.
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    pub compile_profile_spec_version: String,
    /// Optional because a source `ResolvedCompilePolicy` may be built before a
    /// hint bundle or calibration layer exists. Report sections that embed a
    /// consumed hint/calibration hash can require those fields after validation.
    pub hint_bundle_hash: Option<Hash256>,
    pub compile_request_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum PlacementProfile {
    StrictOnePerBank = 0,
    Budgeted = 1,
    PackedExperts = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementKnob {
    pub profile: PlacementProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementKnobBounds {
    pub max_profile: PlacementProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ObservabilityMode {
    Invariant,
    Flexible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ProbeCollectionLevel {
    RequiredOnly = 0,
    Operational = 1,
    Verbose = 2,
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceDemotionLevel {
    #[default]
    None = 0,
    DropBestEffort = 1,
    DropDiagnosticAndBestEffort = 2,
    RequiredOnly = 3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnob {
    pub observability: ObservabilityMode,
    #[serde(default)]
    pub trace_demotion: TraceDemotionLevel,
    pub probe_level: ProbeCollectionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnobBounds {
    #[serde(default = "default_max_trace_demotion")]
    pub max_trace_demotion: TraceDemotionLevel,
    pub max_probe_level: ProbeCollectionLevel,
}

const fn default_max_trace_demotion() -> TraceDemotionLevel {
    TraceDemotionLevel::RequiredOnly
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ReductionPlanCeiling {
    ExactOnly = 0,
    Conservative = 1,
    Adaptive = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeKnob {
    pub reduction_ceiling: ReductionPlanCeiling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RangeKnobBounds {
    pub max_reduction_ceiling: ReductionPlanCeiling,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StorageMaterialization {
    PreserveAll = 0,
    RecomputePureValues = 1,
    SpillColdValues = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageKnob {
    pub materialization: StorageMaterialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageKnobBounds {
    pub max_materialization: StorageMaterialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramPageAggression {
    Preserve = 0,
    PackCold = 1,
    MinimizeResident = 2,
}

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum SramSpillPolicy {
    #[default]
    NoSpill = 0,
    SpillOnPressure = 1,
    SpillEager = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnob {
    pub page_aggression: SramPageAggression,
    #[serde(default)]
    pub spill_policy: SramSpillPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnobBounds {
    pub max_page_aggression: SramPageAggression,
    #[serde(default = "default_max_spill_policy")]
    pub max_spill_policy: SramSpillPolicy,
}

const fn default_max_spill_policy() -> SramSpillPolicy {
    SramSpillPolicy::SpillEager
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(clippy::enum_variant_names)]
pub enum RomKernelResidencyBias {
    PreferCommonBank = 0,
    PreferExpertBank = 1,
    PreferWramOverlay = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RomKernelDuplicationBias {
    Share = 0,
    DuplicateHot = 1,
    DuplicateAllFit = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowKnob {
    pub kernel_residency_bias: RomKernelResidencyBias,
    pub kernel_duplication_bias: RomKernelDuplicationBias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomWindowKnobBounds {
    pub max_kernel_residency_bias: RomKernelResidencyBias,
    pub max_kernel_duplication_bias: RomKernelDuplicationBias,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum OverlayPromotion {
    Disabled = 0,
    TinyLuts = 1,
    EligibleKernels = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayKnob {
    pub promotion: OverlayPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverlayKnobBounds {
    pub max_promotion: OverlayPromotion,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleTileSearch {
    Fixed = 0,
    Local = 1,
    ProfileGuided = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleSliceCoarsening {
    Fine = 0,
    Balanced = 1,
    Coarse = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ScheduleResourcePressure {
    Conservative = 0,
    Balanced = 1,
    FitFirst = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageIterationLimits {
    pub range_plan: u8,
    pub storage_plan: u8,
    pub sram_page_plan: u8,
    pub rom_window_plan: u8,
    pub overlay_plan: u8,
    pub arena_plan: u8,
    pub gb_sched_ir: u8,
    pub resource_state_validation: u8,
}

impl StageIterationLimits {
    #[must_use]
    pub const fn uniform(limit: u8) -> Self {
        Self {
            range_plan: limit,
            storage_plan: limit,
            sram_page_plan: limit,
            rom_window_plan: limit,
            overlay_plan: limit,
            arena_plan: limit,
            gb_sched_ir: limit,
            resource_state_validation: limit,
        }
    }

    #[must_use]
    pub const fn all_le(self, other: Self) -> bool {
        self.range_plan <= other.range_plan
            && self.storage_plan <= other.storage_plan
            && self.sram_page_plan <= other.sram_page_plan
            && self.rom_window_plan <= other.rom_window_plan
            && self.overlay_plan <= other.overlay_plan
            && self.arena_plan <= other.arena_plan
            && self.gb_sched_ir <= other.gb_sched_ir
            && self.resource_state_validation <= other.resource_state_validation
    }
}

impl Default for StageIterationLimits {
    fn default() -> Self {
        Self::uniform(4)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnob {
    pub tile_search: ScheduleTileSearch,
    pub slice_coarsening: ScheduleSliceCoarsening,
    pub resource_pressure: ScheduleResourcePressure,
    #[serde(default)]
    pub pressure_thresholds: ResourcePressureThresholds,
    #[serde(default)]
    pub stage_iteration_ceilings: StageIterationLimits,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnobBounds {
    pub max_tile_search: ScheduleTileSearch,
    pub max_slice_coarsening: ScheduleSliceCoarsening,
    pub max_resource_pressure: ScheduleResourcePressure,
    #[serde(default = "default_max_resource_pressure_thresholds")]
    pub max_pressure_thresholds: ResourcePressureThresholds,
    #[serde(default = "default_max_stage_iteration_ceilings")]
    pub max_stage_iteration_ceilings: StageIterationLimits,
}

const fn default_max_resource_pressure_thresholds() -> ResourcePressureThresholds {
    ResourcePressureThresholds {
        wram_hot: PressureLimit {
            soft: 8 * 1024,
            hard: 8 * 1024,
        },
        hram_hot: PressureLimit {
            soft: 127,
            hard: 127,
        },
        bank0_rom: PressureLimit {
            soft: 16 * 1024,
            hard: 16 * 1024,
        },
        switchable_rom_window: PressureLimit {
            soft: 16 * 1024,
            hard: 16 * 1024,
        },
        sram_window: PressureLimit {
            soft: 8 * 1024,
            hard: 8 * 1024,
        },
        slice_cycles: PressureLimit {
            soft: u64::MAX / 2,
            hard: u64::MAX / 2,
        },
        interrupt_latency: PressureLimit {
            soft: u64::MAX / 2,
            hard: u64::MAX / 2,
        },
        trace_bytes_per_frame: PressureLimit {
            soft: u16::MAX,
            hard: u16::MAX,
        },
        persist_bytes_per_frame: PressureLimit {
            soft: u16::MAX,
            hard: u16::MAX,
        },
        overlay_installs_per_frame: PressureLimit {
            soft: u8::MAX,
            hard: u8::MAX,
        },
        bank_switches_per_token: PressureLimit {
            soft: u16::MAX,
            hard: u16::MAX,
        },
        sram_page_switches_per_token: PressureLimit {
            soft: u16::MAX,
            hard: u16::MAX,
        },
    }
}

const fn default_max_stage_iteration_ceilings() -> StageIterationLimits {
    StageIterationLimits::uniform(u8::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ConstraintValue {
    PlacementProfile { value: PlacementProfile },
    ObservabilityMode { value: ObservabilityMode },
    U16 { value: u16 },
    U32 { value: u32 },
    Bool { value: bool },
    Text { value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum CompilerFeature {
    ArtifactValidation,
    PolicyResolution,
    QuantGraphBudgetSource,
    StaticBudgetReport,
    Ternary2Quant,
    Binary1Quant,
    SparseTernaryBitplanes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RuntimeMode {
    Interactive,
    Steady,
    Trace,
    Safe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndFault,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TraceBudget {
    pub max_events_per_slice: u16,
    pub max_bytes_per_frame: u16,
    pub drop_policy: TraceDropPolicy,
}

pub trait MonotoneKnob {
    #[must_use]
    fn rank(&self) -> u8 {
        0
    }

    fn is_monotone_successor_of(&self, previous: &Self) -> bool;

    #[must_use]
    fn is_monotone_advance(from: &Self, to: &Self) -> bool {
        from.rank() <= to.rank()
    }

    #[must_use]
    fn is_strict_advance(from: &Self, to: &Self) -> bool {
        from.rank() < to.rank()
    }
}

macro_rules! monotone_enum {
    ($ty:ty) => {
        impl MonotoneKnob for $ty {
            fn rank(&self) -> u8 {
                *self as u8
            }

            fn is_monotone_successor_of(&self, previous: &Self) -> bool {
                self >= previous
            }
        }
    };
}

monotone_enum!(PlacementProfile);
monotone_enum!(TraceDemotionLevel);
monotone_enum!(ProbeCollectionLevel);
monotone_enum!(ReductionPlanCeiling);
monotone_enum!(StorageMaterialization);
monotone_enum!(SramPageAggression);
monotone_enum!(SramSpillPolicy);
monotone_enum!(RomKernelResidencyBias);
monotone_enum!(RomKernelDuplicationBias);
monotone_enum!(OverlayPromotion);
monotone_enum!(ScheduleTileSearch);
monotone_enum!(ScheduleSliceCoarsening);
monotone_enum!(ScheduleResourcePressure);
monotone_enum!(KernelResidency);

impl MonotoneKnob for PlacementKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.profile.is_monotone_successor_of(&previous.profile)
    }
}

impl MonotoneKnob for PlacementKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_profile <= previous.max_profile
    }
}

impl MonotoneKnob for ObservationKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.trace_demotion
            .is_monotone_successor_of(&previous.trace_demotion)
            && self
                .probe_level
                .is_monotone_successor_of(&previous.probe_level)
    }
}

impl MonotoneKnob for ObservationKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_trace_demotion <= previous.max_trace_demotion
            && self.max_probe_level <= previous.max_probe_level
    }
}

impl MonotoneKnob for RangeKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.reduction_ceiling
            .is_monotone_successor_of(&previous.reduction_ceiling)
    }
}

impl MonotoneKnob for RangeKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_reduction_ceiling <= previous.max_reduction_ceiling
    }
}

impl MonotoneKnob for StorageKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.materialization
            .is_monotone_successor_of(&previous.materialization)
    }
}

impl MonotoneKnob for StorageKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_materialization <= previous.max_materialization
    }
}

impl MonotoneKnob for SramKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.page_aggression
            .is_monotone_successor_of(&previous.page_aggression)
            && self
                .spill_policy
                .is_monotone_successor_of(&previous.spill_policy)
    }
}

impl MonotoneKnob for SramKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_page_aggression <= previous.max_page_aggression
            && self.max_spill_policy <= previous.max_spill_policy
    }
}

impl MonotoneKnob for RomWindowKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.kernel_residency_bias
            .is_monotone_successor_of(&previous.kernel_residency_bias)
            && self
                .kernel_duplication_bias
                .is_monotone_successor_of(&previous.kernel_duplication_bias)
    }
}

impl MonotoneKnob for RomWindowKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_kernel_residency_bias <= previous.max_kernel_residency_bias
            && self.max_kernel_duplication_bias <= previous.max_kernel_duplication_bias
    }
}

impl MonotoneKnob for OverlayKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.promotion.is_monotone_successor_of(&previous.promotion)
    }
}

impl MonotoneKnob for OverlayKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_promotion <= previous.max_promotion
    }
}

impl MonotoneKnob for ScheduleKnob {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.tile_search
            .is_monotone_successor_of(&previous.tile_search)
            && self
                .slice_coarsening
                .is_monotone_successor_of(&previous.slice_coarsening)
            && self
                .resource_pressure
                .is_monotone_successor_of(&previous.resource_pressure)
            && self
                .pressure_thresholds
                .all_ge(previous.pressure_thresholds)
            && self
                .stage_iteration_ceilings
                .all_le(previous.stage_iteration_ceilings)
    }
}

impl MonotoneKnob for ScheduleKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_tile_search <= previous.max_tile_search
            && self.max_slice_coarsening <= previous.max_slice_coarsening
            && self.max_resource_pressure <= previous.max_resource_pressure
            && self
                .max_pressure_thresholds
                .all_le(previous.max_pressure_thresholds)
            && self
                .max_stage_iteration_ceilings
                .all_le(previous.max_stage_iteration_ceilings)
    }
}

impl KnobLockSet {
    #[must_use]
    pub fn is_locked(&self, knob: CompileKnobId) -> bool {
        self.locked.contains(&knob) || self.locked.contains(&knob.top_level())
    }
}

impl KnobDelta {
    #[must_use]
    pub const fn knob_id(&self) -> CompileKnobId {
        match self {
            Self::AdvancePlacementProfile { .. } => CompileKnobId::PlacementProfile,
            Self::SetTraceDemotion { .. } => CompileKnobId::ObservationTraceDemotion,
            Self::DisableOptionalProbes { .. } => CompileKnobId::ObservationProbeSelection,
            Self::RaiseReductionCeiling { .. } => CompileKnobId::RangeReductionCeiling,
            Self::PromoteRecomputeLevel { .. } => CompileKnobId::StorageRecomputePromotion,
            Self::ForceRecompute { .. } => CompileKnobId::StorageMaterializationOverrides,
            Self::AdvanceSramPageAggression { .. } => CompileKnobId::SramPageAggression,
            Self::AdvanceSramSpillPolicy { .. } => CompileKnobId::SramSpillPolicy,
            Self::AdvanceKernelResidencyBias { .. } => CompileKnobId::RomKernelResidencyBias,
            Self::AdvanceKernelDuplicationBias { .. } => CompileKnobId::RomKernelDuplicationBias,
            Self::ForceKernelResidency { .. } => CompileKnobId::RomKernelResidencyOverrides,
            Self::PromoteOverlay { .. } => CompileKnobId::OverlayPromotion,
            Self::NarrowTileClasses { .. } => CompileKnobId::ScheduleTileSearch,
            Self::SetSliceCoarsening { .. } => CompileKnobId::ScheduleSliceCoarsening,
            Self::UpdatePressureThreshold { .. } => CompileKnobId::ScheduleResourcePressure,
        }
    }
}

pub fn check_delta_admissible(
    delta: &KnobDelta,
    current: &CompileKnobs,
    policy: &RepairPolicy,
    observability: ObservabilityMode,
) -> Result<(), DeltaRejection> {
    check_delta_admissible_with_recompute_purity(
        delta,
        current,
        policy,
        observability,
        &RecomputePurityFacts::default(),
    )
}

pub fn check_delta_admissible_with_recompute_purity(
    delta: &KnobDelta,
    current: &CompileKnobs,
    policy: &RepairPolicy,
    observability: ObservabilityMode,
    recompute_purity: &RecomputePurityFacts,
) -> Result<(), DeltaRejection> {
    let knob = delta.knob_id();
    if current.locks.is_locked(knob) {
        return Err(DeltaRejection::KnobLocked { knob });
    }

    if let Some(toggle) = disabled_policy_toggle(delta, policy) {
        return Err(DeltaRejection::PolicyToggleDisabled {
            knob,
            toggle: toggle.to_owned(),
        });
    }

    check_delta_bounds(delta, current)?;
    check_delta_monotone(delta, current)?;
    check_recompute_purity(delta, recompute_purity)?;

    if observability == ObservabilityMode::Invariant
        && matches!(
            knob,
            CompileKnobId::ObservationTraceDemotion | CompileKnobId::ObservationProbeSelection
        )
    {
        return Err(DeltaRejection::InvariantObservabilityViolation { knob });
    }

    Ok(())
}

fn check_recompute_purity(
    delta: &KnobDelta,
    recompute_purity: &RecomputePurityFacts,
) -> Result<(), DeltaRejection> {
    let KnobDelta::ForceRecompute { values } = delta else {
        return Ok(());
    };

    for value in values {
        if !recompute_purity.allows(value) {
            return Err(DeltaRejection::EffectfulRecompute {
                value: value.clone(),
            });
        }
    }

    Ok(())
}

#[must_use]
pub fn f_b16_profile_lock_set(profile: RepairPolicyProfile) -> KnobLockSet {
    let locked = match profile {
        RepairPolicyProfile::Bringup | RepairPolicyProfile::BringupFirstFit => BTreeSet::from([
            CompileKnobId::PlacementProfile,
            CompileKnobId::ObservationTraceDemotion,
            CompileKnobId::ObservationProbeSelection,
            CompileKnobId::RangeReductionCeiling,
            CompileKnobId::StorageRecomputePromotion,
            CompileKnobId::StorageMaterializationOverrides,
            CompileKnobId::SramPageAggression,
            CompileKnobId::SramSpillPolicy,
            CompileKnobId::RomKernelResidencyBias,
            CompileKnobId::RomKernelDuplicationBias,
            CompileKnobId::RomKernelResidencyOverrides,
            CompileKnobId::OverlayPromotion,
            CompileKnobId::ScheduleResourcePressure,
            CompileKnobId::StageIterationCeilings,
        ]),
        RepairPolicyProfile::Default | RepairPolicyProfile::Recovery => BTreeSet::from([
            CompileKnobId::ScheduleResourcePressure,
            CompileKnobId::StageIterationCeilings,
        ]),
        RepairPolicyProfile::TraceInvariant => f_b16_refinement_knob_ids().into_iter().collect(),
    };
    KnobLockSet { locked }
}

#[must_use]
pub const fn f_b16_refinement_knob_ids() -> [CompileKnobId; 16] {
    [
        CompileKnobId::PlacementProfile,
        CompileKnobId::ObservationTraceDemotion,
        CompileKnobId::ObservationProbeSelection,
        CompileKnobId::RangeReductionCeiling,
        CompileKnobId::StorageRecomputePromotion,
        CompileKnobId::StorageMaterializationOverrides,
        CompileKnobId::SramPageAggression,
        CompileKnobId::SramSpillPolicy,
        CompileKnobId::RomKernelResidencyBias,
        CompileKnobId::RomKernelDuplicationBias,
        CompileKnobId::RomKernelResidencyOverrides,
        CompileKnobId::OverlayPromotion,
        CompileKnobId::ScheduleTileSearch,
        CompileKnobId::ScheduleSliceCoarsening,
        CompileKnobId::ScheduleResourcePressure,
        CompileKnobId::StageIterationCeilings,
    ]
}

fn disabled_policy_toggle(delta: &KnobDelta, policy: &RepairPolicy) -> Option<&'static str> {
    match delta {
        KnobDelta::AdvancePlacementProfile { .. } if !policy.allow_placement_profile_fallback => {
            Some("allow_placement_profile_fallback")
        }
        KnobDelta::SetTraceDemotion { .. } | KnobDelta::DisableOptionalProbes { .. }
            if !policy.allow_trace_demotion =>
        {
            Some("allow_trace_demotion")
        }
        KnobDelta::PromoteRecomputeLevel { .. } | KnobDelta::ForceRecompute { .. }
            if !policy.allow_recompute_promotion =>
        {
            Some("allow_recompute_promotion")
        }
        KnobDelta::PromoteOverlay { .. } if !policy.allow_overlay_promotion => {
            Some("allow_overlay_promotion")
        }
        KnobDelta::ForceKernelResidency {
            to: KernelResidency::WramOverlay,
            ..
        } if !policy.allow_overlay_promotion => Some("allow_overlay_promotion"),
        _ => None,
    }
}

fn check_delta_bounds(delta: &KnobDelta, current: &CompileKnobs) -> Result<(), DeltaRejection> {
    match delta {
        KnobDelta::AdvancePlacementProfile { to } => check_ordered_bound(
            CompileKnobId::PlacementProfile,
            to,
            &current.bounds.placement.max_profile,
        ),
        KnobDelta::SetTraceDemotion { to } => check_ordered_bound(
            CompileKnobId::ObservationTraceDemotion,
            to,
            &current.bounds.observation.max_trace_demotion,
        ),
        KnobDelta::RaiseReductionCeiling { to, .. } => check_ordered_bound(
            CompileKnobId::RangeReductionCeiling,
            to,
            &current.bounds.range.max_reduction_ceiling,
        ),
        KnobDelta::PromoteRecomputeLevel { to } => check_ordered_bound(
            CompileKnobId::StorageRecomputePromotion,
            to,
            &current.bounds.storage.max_materialization,
        ),
        KnobDelta::AdvanceSramPageAggression { to } => check_ordered_bound(
            CompileKnobId::SramPageAggression,
            to,
            &current.bounds.sram.max_page_aggression,
        ),
        KnobDelta::AdvanceSramSpillPolicy { to } => check_ordered_bound(
            CompileKnobId::SramSpillPolicy,
            to,
            &current.bounds.sram.max_spill_policy,
        ),
        KnobDelta::AdvanceKernelResidencyBias { to } => check_ordered_bound(
            CompileKnobId::RomKernelResidencyBias,
            to,
            &current.bounds.rom_window.max_kernel_residency_bias,
        ),
        KnobDelta::AdvanceKernelDuplicationBias { to } => check_ordered_bound(
            CompileKnobId::RomKernelDuplicationBias,
            to,
            &current.bounds.rom_window.max_kernel_duplication_bias,
        ),
        KnobDelta::PromoteOverlay { to } => check_ordered_bound(
            CompileKnobId::OverlayPromotion,
            to,
            &current.bounds.overlay.max_promotion,
        ),
        KnobDelta::SetSliceCoarsening { to } => check_ordered_bound(
            CompileKnobId::ScheduleSliceCoarsening,
            to,
            &current.bounds.schedule.max_slice_coarsening,
        ),
        KnobDelta::DisableOptionalProbes { .. }
        | KnobDelta::ForceRecompute { .. }
        | KnobDelta::ForceKernelResidency { .. }
        | KnobDelta::NarrowTileClasses { .. } => Ok(()),
        KnobDelta::UpdatePressureThreshold { update } => {
            check_pressure_update_limit(update)?;
            check_pressure_update_bound(update, &current.bounds.schedule.max_pressure_thresholds)
        }
    }
}

fn check_pressure_update_limit(update: &ResourcePressureUpdate) -> Result<(), DeltaRejection> {
    match update {
        ResourcePressureUpdate::WramHot { limit } => check_pressure_limit("wram_hot", limit),
        ResourcePressureUpdate::HramHot { limit } => check_pressure_limit("hram_hot", limit),
        ResourcePressureUpdate::Bank0Rom { limit } => check_pressure_limit("bank0_rom", limit),
        ResourcePressureUpdate::SwitchableRomWindow { limit } => {
            check_pressure_limit("switchable_rom_window", limit)
        }
        ResourcePressureUpdate::SramWindow { limit } => check_pressure_limit("sram_window", limit),
        ResourcePressureUpdate::SliceCycles { limit } => {
            check_pressure_limit("slice_cycles", limit)
        }
        ResourcePressureUpdate::InterruptLatency { limit } => {
            check_pressure_limit("interrupt_latency", limit)
        }
        ResourcePressureUpdate::TraceBytesPerFrame { limit } => {
            check_pressure_limit("trace_bytes_per_frame", limit)
        }
        ResourcePressureUpdate::PersistBytesPerFrame { limit } => {
            check_pressure_limit("persist_bytes_per_frame", limit)
        }
        ResourcePressureUpdate::OverlayInstallsPerFrame { limit } => {
            check_pressure_limit("overlay_installs_per_frame", limit)
        }
        ResourcePressureUpdate::BankSwitchesPerToken { limit } => {
            check_pressure_limit("bank_switches_per_token", limit)
        }
        ResourcePressureUpdate::SramPageSwitchesPerToken { limit } => {
            check_pressure_limit("sram_page_switches_per_token", limit)
        }
    }
}

fn check_pressure_limit<T>(
    field: &'static str,
    limit: &PressureLimit<T>,
) -> Result<(), DeltaRejection>
where
    T: PartialOrd + fmt::Debug,
{
    if limit.soft <= limit.hard {
        Ok(())
    } else {
        Err(DeltaRejection::BeyondBounds {
            knob: CompileKnobId::ScheduleResourcePressure,
            attempted: format!("{field}.soft={:?}", limit.soft),
            max: format!("{field}.hard={:?}; invariant soft <= hard", limit.hard),
        })
    }
}

fn check_pressure_update_bound(
    update: &ResourcePressureUpdate,
    max: &ResourcePressureThresholds,
) -> Result<(), DeltaRejection> {
    match update {
        ResourcePressureUpdate::WramHot { limit } => {
            check_pressure_limit_bound("wram_hot", limit, &max.wram_hot)
        }
        ResourcePressureUpdate::HramHot { limit } => {
            check_pressure_limit_bound("hram_hot", limit, &max.hram_hot)
        }
        ResourcePressureUpdate::Bank0Rom { limit } => {
            check_pressure_limit_bound("bank0_rom", limit, &max.bank0_rom)
        }
        ResourcePressureUpdate::SwitchableRomWindow { limit } => {
            check_pressure_limit_bound("switchable_rom_window", limit, &max.switchable_rom_window)
        }
        ResourcePressureUpdate::SramWindow { limit } => {
            check_pressure_limit_bound("sram_window", limit, &max.sram_window)
        }
        ResourcePressureUpdate::SliceCycles { limit } => {
            check_pressure_limit_bound("slice_cycles", limit, &max.slice_cycles)
        }
        ResourcePressureUpdate::InterruptLatency { limit } => {
            check_pressure_limit_bound("interrupt_latency", limit, &max.interrupt_latency)
        }
        ResourcePressureUpdate::TraceBytesPerFrame { limit } => {
            check_pressure_limit_bound("trace_bytes_per_frame", limit, &max.trace_bytes_per_frame)
        }
        ResourcePressureUpdate::PersistBytesPerFrame { limit } => check_pressure_limit_bound(
            "persist_bytes_per_frame",
            limit,
            &max.persist_bytes_per_frame,
        ),
        ResourcePressureUpdate::OverlayInstallsPerFrame { limit } => check_pressure_limit_bound(
            "overlay_installs_per_frame",
            limit,
            &max.overlay_installs_per_frame,
        ),
        ResourcePressureUpdate::BankSwitchesPerToken { limit } => check_pressure_limit_bound(
            "bank_switches_per_token",
            limit,
            &max.bank_switches_per_token,
        ),
        ResourcePressureUpdate::SramPageSwitchesPerToken { limit } => check_pressure_limit_bound(
            "sram_page_switches_per_token",
            limit,
            &max.sram_page_switches_per_token,
        ),
    }
}

fn check_pressure_limit_bound<T>(
    field: &'static str,
    attempted: &PressureLimit<T>,
    max: &PressureLimit<T>,
) -> Result<(), DeltaRejection>
where
    T: PartialOrd + fmt::Debug,
{
    if attempted.soft <= max.soft && attempted.hard <= max.hard {
        Ok(())
    } else {
        Err(DeltaRejection::BeyondBounds {
            knob: CompileKnobId::ScheduleResourcePressure,
            attempted: format!(
                "{field}.soft={:?}, {field}.hard={:?}",
                attempted.soft, attempted.hard
            ),
            max: format!("{field}.soft={:?}, {field}.hard={:?}", max.soft, max.hard),
        })
    }
}

fn check_pressure_update_monotone(
    update: &ResourcePressureUpdate,
    current: &ResourcePressureThresholds,
) -> Result<(), DeltaRejection> {
    match update {
        ResourcePressureUpdate::WramHot { limit } => {
            check_pressure_limit_monotone("wram_hot", limit, &current.wram_hot)
        }
        ResourcePressureUpdate::HramHot { limit } => {
            check_pressure_limit_monotone("hram_hot", limit, &current.hram_hot)
        }
        ResourcePressureUpdate::Bank0Rom { limit } => {
            check_pressure_limit_monotone("bank0_rom", limit, &current.bank0_rom)
        }
        ResourcePressureUpdate::SwitchableRomWindow { limit } => check_pressure_limit_monotone(
            "switchable_rom_window",
            limit,
            &current.switchable_rom_window,
        ),
        ResourcePressureUpdate::SramWindow { limit } => {
            check_pressure_limit_monotone("sram_window", limit, &current.sram_window)
        }
        ResourcePressureUpdate::SliceCycles { limit } => {
            check_pressure_limit_monotone("slice_cycles", limit, &current.slice_cycles)
        }
        ResourcePressureUpdate::InterruptLatency { limit } => {
            check_pressure_limit_monotone("interrupt_latency", limit, &current.interrupt_latency)
        }
        ResourcePressureUpdate::TraceBytesPerFrame { limit } => check_pressure_limit_monotone(
            "trace_bytes_per_frame",
            limit,
            &current.trace_bytes_per_frame,
        ),
        ResourcePressureUpdate::PersistBytesPerFrame { limit } => check_pressure_limit_monotone(
            "persist_bytes_per_frame",
            limit,
            &current.persist_bytes_per_frame,
        ),
        ResourcePressureUpdate::OverlayInstallsPerFrame { limit } => check_pressure_limit_monotone(
            "overlay_installs_per_frame",
            limit,
            &current.overlay_installs_per_frame,
        ),
        ResourcePressureUpdate::BankSwitchesPerToken { limit } => check_pressure_limit_monotone(
            "bank_switches_per_token",
            limit,
            &current.bank_switches_per_token,
        ),
        ResourcePressureUpdate::SramPageSwitchesPerToken { limit } => {
            check_pressure_limit_monotone(
                "sram_page_switches_per_token",
                limit,
                &current.sram_page_switches_per_token,
            )
        }
    }
}

fn check_pressure_limit_monotone<T>(
    field: &'static str,
    attempted: &PressureLimit<T>,
    current: &PressureLimit<T>,
) -> Result<(), DeltaRejection>
where
    T: PartialOrd + PartialEq + fmt::Debug,
{
    if attempted.soft >= current.soft
        && attempted.hard >= current.hard
        && (attempted.soft != current.soft || attempted.hard != current.hard)
    {
        Ok(())
    } else {
        Err(DeltaRejection::NotMonotone {
            knob: CompileKnobId::ScheduleResourcePressure,
            current: format!(
                "{field}.soft={:?}, {field}.hard={:?}",
                current.soft, current.hard
            ),
            attempted: format!(
                "{field}.soft={:?}, {field}.hard={:?}",
                attempted.soft, attempted.hard
            ),
        })
    }
}

fn check_ordered_bound<T>(knob: CompileKnobId, attempted: &T, max: &T) -> Result<(), DeltaRejection>
where
    T: MonotoneKnob + std::fmt::Debug,
{
    if attempted.rank() <= max.rank() {
        Ok(())
    } else {
        Err(DeltaRejection::BeyondBounds {
            knob,
            attempted: format!("{attempted:?}"),
            max: format!("{max:?}"),
        })
    }
}

fn check_delta_monotone(delta: &KnobDelta, current: &CompileKnobs) -> Result<(), DeltaRejection> {
    match delta {
        KnobDelta::AdvancePlacementProfile { to } => check_strict_advance(
            CompileKnobId::PlacementProfile,
            &current.global.placement.profile,
            to,
        ),
        KnobDelta::SetTraceDemotion { to } => check_strict_advance(
            CompileKnobId::ObservationTraceDemotion,
            &current.global.observation.trace_demotion,
            to,
        ),
        KnobDelta::RaiseReductionCeiling { to, .. } => check_strict_advance(
            CompileKnobId::RangeReductionCeiling,
            &current.global.range.reduction_ceiling,
            to,
        ),
        KnobDelta::PromoteRecomputeLevel { to } => check_strict_advance(
            CompileKnobId::StorageRecomputePromotion,
            &current.global.storage.materialization,
            to,
        ),
        KnobDelta::AdvanceSramPageAggression { to } => check_strict_advance(
            CompileKnobId::SramPageAggression,
            &current.global.sram.page_aggression,
            to,
        ),
        KnobDelta::AdvanceSramSpillPolicy { to } => check_strict_advance(
            CompileKnobId::SramSpillPolicy,
            &current.global.sram.spill_policy,
            to,
        ),
        KnobDelta::AdvanceKernelResidencyBias { to } => check_strict_advance(
            CompileKnobId::RomKernelResidencyBias,
            &current.global.rom_window.kernel_residency_bias,
            to,
        ),
        KnobDelta::AdvanceKernelDuplicationBias { to } => check_strict_advance(
            CompileKnobId::RomKernelDuplicationBias,
            &current.global.rom_window.kernel_duplication_bias,
            to,
        ),
        KnobDelta::PromoteOverlay { to } => check_strict_advance(
            CompileKnobId::OverlayPromotion,
            &current.global.overlay.promotion,
            to,
        ),
        KnobDelta::SetSliceCoarsening { to } => check_strict_advance(
            CompileKnobId::ScheduleSliceCoarsening,
            &current.global.schedule.slice_coarsening,
            to,
        ),
        KnobDelta::DisableOptionalProbes { probes } => {
            if probes.is_empty() {
                Err(DeltaRejection::NotMonotone {
                    knob: CompileKnobId::ObservationProbeSelection,
                    current: "disabled_optional_probes".to_owned(),
                    attempted: "empty".to_owned(),
                })
            } else {
                Ok(())
            }
        }
        KnobDelta::ForceRecompute { values } => {
            if values.is_empty() {
                Err(DeltaRejection::NotMonotone {
                    knob: CompileKnobId::StorageMaterializationOverrides,
                    current: "forced_recompute".to_owned(),
                    attempted: "empty".to_owned(),
                })
            } else {
                Ok(())
            }
        }
        KnobDelta::ForceKernelResidency { .. } => Ok(()),
        KnobDelta::UpdatePressureThreshold { update } => {
            check_pressure_update_monotone(update, &current.global.schedule.pressure_thresholds)
        }
        KnobDelta::NarrowTileClasses { remaining, .. } => {
            if remaining.is_empty() {
                Err(DeltaRejection::NotMonotone {
                    knob: CompileKnobId::ScheduleTileSearch,
                    current: "tile_class_overrides".to_owned(),
                    attempted: "empty".to_owned(),
                })
            } else {
                Ok(())
            }
        }
    }
}

fn check_strict_advance<T>(
    knob: CompileKnobId,
    current: &T,
    attempted: &T,
) -> Result<(), DeltaRejection>
where
    T: MonotoneKnob + std::fmt::Debug,
{
    if T::is_strict_advance(current, attempted) {
        Ok(())
    } else {
        Err(DeltaRejection::NotMonotone {
            knob,
            current: format!("{current:?}"),
            attempted: format!("{attempted:?}"),
        })
    }
}

#[must_use]
pub const fn canonical_default_bounds_fixture() -> CompileKnobBounds {
    CompileKnobBounds {
        placement: PlacementKnobBounds {
            max_profile: PlacementProfile::PackedExperts,
        },
        observation: ObservationKnobBounds {
            max_trace_demotion: TraceDemotionLevel::RequiredOnly,
            max_probe_level: ProbeCollectionLevel::Verbose,
        },
        range: RangeKnobBounds {
            max_reduction_ceiling: ReductionPlanCeiling::Adaptive,
        },
        storage: StorageKnobBounds {
            max_materialization: StorageMaterialization::SpillColdValues,
        },
        sram: SramKnobBounds {
            max_page_aggression: SramPageAggression::MinimizeResident,
            max_spill_policy: SramSpillPolicy::SpillEager,
        },
        rom_window: RomWindowKnobBounds {
            max_kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
            max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateAllFit,
        },
        overlay: OverlayKnobBounds {
            max_promotion: OverlayPromotion::EligibleKernels,
        },
        schedule: ScheduleKnobBounds {
            max_tile_search: ScheduleTileSearch::ProfileGuided,
            max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
            max_resource_pressure: ScheduleResourcePressure::FitFirst,
            max_pressure_thresholds: default_max_resource_pressure_thresholds(),
            max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fmt;
    use std::sync::{Arc, Mutex, OnceLock};

    use super::*;
    use crate::objective::{CompileObjective, RiskPolicy, ServiceLevelObjective};
    use crate::repair::RepairPolicyProfile;
    use crate::risk::{CalibrationConfidenceClass, CalibrationConfidenceRequirement};
    use tracing_subscriber::filter::LevelFilter;
    use tracing_subscriber::prelude::*;

    fn objective_fixture() -> CompileObjective {
        CompileObjective {
            service: Some(ServiceLevelObjective {
                max_first_token_cycles_p95: Some(3_000),
                max_checkpoint_gap_cycles_p95: None,
                max_resume_latency_cycles_p95: Some(1_000),
                max_ui_jitter_frames_p99: Some(1),
            }),
            max_cycles_per_token: Some(8_000),
            max_bank_switches_per_token: Some(5),
            max_sram_page_switches_per_token: Some(1),
            min_sustained_throughput_tokens_per_megacycle: None,
            min_ui_headroom_pct: 9,
            max_rom_bytes: Some(512 * 1024),
            risk: RiskPolicy {
                cycle_quantile: 95,
                switch_quantile: 99,
                calibration_confidence_requirement: CalibrationConfidenceRequirement::AtLeast {
                    class: CalibrationConfidenceClass::Weak,
                },
                fallback_profile: None,
                fallback_runtime_mode: Some(RuntimeMode::Safe),
            },
        }
    }

    fn values_fixture() -> CompileKnobValues {
        CompileKnobValues {
            placement: PlacementKnob {
                profile: PlacementProfile::Budgeted,
            },
            observation: ObservationKnob {
                observability: ObservabilityMode::Invariant,
                trace_demotion: TraceDemotionLevel::None,
                probe_level: ProbeCollectionLevel::Operational,
            },
            range: RangeKnob {
                reduction_ceiling: ReductionPlanCeiling::Conservative,
            },
            storage: StorageKnob {
                materialization: StorageMaterialization::RecomputePureValues,
            },
            sram: SramKnob {
                page_aggression: SramPageAggression::PackCold,
                spill_policy: SramSpillPolicy::SpillOnPressure,
            },
            rom_window: RomWindowKnob {
                kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            },
            overlay: OverlayKnob {
                promotion: OverlayPromotion::TinyLuts,
            },
            schedule: ScheduleKnob {
                tile_search: ScheduleTileSearch::Local,
                slice_coarsening: ScheduleSliceCoarsening::Balanced,
                resource_pressure: ScheduleResourcePressure::Balanced,
                pressure_thresholds: ResourcePressureThresholds::default(),
                stage_iteration_ceilings: StageIterationLimits::uniform(4),
            },
        }
    }

    fn compile_knobs_fixture() -> CompileKnobs {
        CompileKnobs {
            global: values_fixture(),
            bounds: canonical_default_bounds_fixture(),
            locks: KnobLockSet {
                locked: BTreeSet::from([CompileKnobId::RomWindow]),
            },
            overrides: CompileKnobOverrides {
                values: CompileKnobPartialValues {
                    placement: Some(PlacementKnob {
                        profile: PlacementProfile::Budgeted,
                    }),
                    ..CompileKnobPartialValues::default()
                },
                bounds: CompileKnobPartialBounds::default(),
                ..CompileKnobOverrides::default()
            },
            provenance: vec![CompileKnobProvenanceEntry {
                path: CompileKnobPath {
                    knob: CompileKnobId::Placement,
                    selector: None,
                    field: Some(FieldPath::from("profile")),
                },
                chain: vec![ConstraintProvenance {
                    source: PolicySource::ProfileDefault,
                    operation: ConstraintOperation::SeedDefault,
                    evidence: vec![EvidenceRef {
                        kind: "ProfileFile".to_owned(),
                        reference: "Bringup.toml".to_owned(),
                        hash: Some(Hash256::from_bytes([5; 32])),
                    }],
                }],
            }],
        }
    }

    fn request_fixture() -> CompileRequest {
        CompileRequest {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: objective_fixture(),
            calibration_set_ref: CalibrationSetRef {
                platform: None,
                kernel: None,
                runtime: None,
            },
            required_features: BTreeSet::from([
                CompilerFeature::ArtifactValidation,
                CompilerFeature::PolicyResolution,
            ]),
            constraint_overrides: Some(CompileKnobOverrides {
                values: CompileKnobPartialValues {
                    placement: Some(PlacementKnob {
                        profile: PlacementProfile::StrictOnePerBank,
                    }),
                    ..CompileKnobPartialValues::default()
                },
                bounds: CompileKnobPartialBounds::default(),
                ..CompileKnobOverrides::default()
            }),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
        }
    }

    fn policy_fixture() -> ResolvedCompilePolicy {
        ResolvedCompilePolicy {
            target: TargetProfileId::from("dmg-mbc5"),
            profile: CompileProfileId::from("Bringup"),
            objective: objective_fixture(),
            effective_constraints: EffectiveConstraints {
                target_caps: canonical_default_bounds_fixture(),
                required_features: BTreeSet::from([CompilerFeature::ArtifactValidation]),
                requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
                runtime_chrome_budget: None,
            },
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 4,
                max_bytes_per_frame: 128,
                drop_policy: TraceDropPolicy::HaltAndFault,
            },
            range_caps: RangeCapsSpec::default_v2(),
            observation_caps: ObservationProfileCaps::default_v2(),
            requested_runtime_modes: BTreeSet::from([RuntimeMode::Interactive]),
            knobs: compile_knobs_fixture(),
            repair: RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
            provenance: PolicyProvenance {
                target_defaults: Hash256::from_bytes([1; 32]),
                profile_defaults: Hash256::from_bytes([2; 32]),
                compile_profile_spec_version: COMPILE_PROFILE_SPEC_VERSION.to_owned(),
                hint_bundle_hash: Some(Hash256::from_bytes([3; 32])),
                compile_request_hash: Hash256::from_bytes([4; 32]),
                calibration_hash: Some(Hash256::from_bytes([5; 32])),
            },
        }
    }

    fn hash_json(byte: u8) -> serde_json::Value {
        serde_json::to_value(Hash256::from_bytes([byte; 32])).expect("hash serializes")
    }

    fn risk_policy_json(class: &str) -> serde_json::Value {
        serde_json::json!({
            "cycle_quantile": 95,
            "switch_quantile": 99,
            "calibration_confidence_requirement": {
                "kind": "AtLeast",
                "class": {"kind": class}
            },
            "fallback_profile": null,
            "fallback_runtime_mode": {"kind": "Safe"}
        })
    }

    fn objective_json() -> serde_json::Value {
        serde_json::json!({
            "service": {
                "max_first_token_cycles_p95": 3000,
                "max_checkpoint_gap_cycles_p95": null,
                "max_resume_latency_cycles_p95": 1000,
                "max_ui_jitter_frames_p99": 1
            },
            "max_cycles_per_token": 8000,
            "max_bank_switches_per_token": 5,
            "max_sram_page_switches_per_token": 1,
            "min_ui_headroom_pct": 9,
            "max_rom_bytes": 524288,
            "risk": risk_policy_json("Weak")
        })
    }

    fn values_json() -> serde_json::Value {
        let mut value = serde_json::json!({
            "placement": {"profile": {"kind": "Budgeted"}},
            "observation": {
                "observability": {"kind": "Invariant"},
                "trace_demotion": {"kind": "None"},
                "probe_level": {"kind": "Operational"}
            },
            "range": {"reduction_ceiling": {"kind": "Conservative"}},
            "storage": {"materialization": {"kind": "RecomputePureValues"}},
            "sram": {
                "page_aggression": {"kind": "PackCold"},
                "spill_policy": {"kind": "SpillOnPressure"}
            },
            "rom_window": {
                "kernel_residency_bias": {"kind": "PreferExpertBank"},
                "kernel_duplication_bias": {"kind": "DuplicateHot"}
            },
            "overlay": {"promotion": {"kind": "TinyLuts"}},
            "schedule": {
                "tile_search": {"kind": "Local"},
                "slice_coarsening": {"kind": "Balanced"},
                "resource_pressure": {"kind": "Balanced"}
            }
        });
        value["schedule"]["pressure_thresholds"] =
            serde_json::to_value(ResourcePressureThresholds::default()).expect("thresholds json");
        value["schedule"]["stage_iteration_ceilings"] =
            serde_json::to_value(StageIterationLimits::uniform(4)).expect("stage limits json");
        value
    }

    fn default_bounds_json() -> serde_json::Value {
        let mut value = serde_json::json!({
            "placement": {"max_profile": {"kind": "PackedExperts"}},
            "observation": {
                "max_trace_demotion": {"kind": "RequiredOnly"},
                "max_probe_level": {"kind": "Verbose"}
            },
            "range": {"max_reduction_ceiling": {"kind": "Adaptive"}},
            "storage": {"max_materialization": {"kind": "SpillColdValues"}},
            "sram": {
                "max_page_aggression": {"kind": "MinimizeResident"},
                "max_spill_policy": {"kind": "SpillEager"}
            },
            "rom_window": {
                "max_kernel_residency_bias": {"kind": "PreferWramOverlay"},
                "max_kernel_duplication_bias": {"kind": "DuplicateAllFit"}
            },
            "overlay": {"max_promotion": {"kind": "EligibleKernels"}},
            "schedule": {
                "max_tile_search": {"kind": "ProfileGuided"},
                "max_slice_coarsening": {"kind": "Coarse"},
                "max_resource_pressure": {"kind": "FitFirst"}
            }
        });
        value["schedule"]["max_pressure_thresholds"] =
            serde_json::to_value(default_max_resource_pressure_thresholds())
                .expect("max thresholds json");
        value["schedule"]["max_stage_iteration_ceilings"] =
            serde_json::to_value(default_max_stage_iteration_ceilings())
                .expect("max stage limits json");
        value
    }

    fn empty_partial_values_json() -> serde_json::Value {
        serde_json::json!({
            "placement": null,
            "observation": null,
            "range": null,
            "storage": null,
            "sram": null,
            "rom_window": null,
            "overlay": null,
            "schedule": null
        })
    }

    fn empty_partial_bounds_json() -> serde_json::Value {
        serde_json::json!({
            "placement": null,
            "observation": null,
            "range": null,
            "storage": null,
            "sram": null,
            "rom_window": null,
            "overlay": null,
            "schedule": null
        })
    }

    fn knobs_json() -> serde_json::Value {
        serde_json::json!({
            "global": values_json(),
            "bounds": default_bounds_json(),
            "locks": {"locked": [{"kind": "RomWindow"}]},
            "overrides": {
                "values": {
                    "placement": {"profile": {"kind": "Budgeted"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "bounds": empty_partial_bounds_json()
            },
            "provenance": [
                {
                    "path": {
                        "knob": {"kind": "Placement"},
                        "selector": null,
                        "field": "profile"
                    },
                    "chain": [
                        {
                            "source": {"kind": "ProfileDefault"},
                            "operation": {"kind": "SeedDefault"},
                            "evidence": [
                                {
                                    "kind": "ProfileFile",
                                    "reference": "Bringup.toml",
                                    "hash": hash_json(5)
                                }
                            ]
                        }
                    ]
                }
            ]
        })
    }

    fn request_json() -> serde_json::Value {
        serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "objective": objective_json(),
            "calibration_set_ref": {
                "platform": null,
                "kernel": null,
                "runtime": null
            },
            "required_features": [
                {"kind": "ArtifactValidation"},
                {"kind": "PolicyResolution"}
            ],
            "constraint_overrides": {
                "values": {
                    "placement": {"profile": {"kind": "StrictOnePerBank"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "bounds": empty_partial_bounds_json()
            },
            "requested_runtime_modes": [{"kind": "Interactive"}]
        })
    }

    fn policy_provenance_json() -> serde_json::Value {
        serde_json::json!({
            "target_defaults": hash_json(1),
            "profile_defaults": hash_json(2),
            "compile_profile_spec_version": "2.0.0",
            "hint_bundle_hash": hash_json(3),
            "compile_request_hash": hash_json(4),
            "calibration_hash": hash_json(5)
        })
    }

    fn policy_json() -> serde_json::Value {
        serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "objective": objective_json(),
            "effective_constraints": {
                "target_caps": default_bounds_json(),
                "required_features": [{"kind": "ArtifactValidation"}],
                "requested_runtime_modes": [{"kind": "Interactive"}],
                "runtime_chrome_budget": null
            },
            "observability": {"kind": "Invariant"},
            "trace_budget": {
                "max_events_per_slice": 4,
                "max_bytes_per_frame": 128,
                "drop_policy": {"kind": "HaltAndFault"}
            },
            "range_caps": {
                "profile_chunk_max": 256,
                "profile_tile_max": 256,
                "profile_tile_min": 16,
                "renorm_strategy": {"kind": "ExactPostBoundaryOnly"}
            },
            "observation_caps": {
                "required_max": null,
                "important_max": 256,
                "diagnostic_max": 256,
                "best_effort_max": 256
            },
            "requested_runtime_modes": [{"kind": "Interactive"}],
            "knobs": knobs_json(),
            "repair": {
                "max_refinement_iters": 1,
                "allow_placement_profile_fallback": false,
                "allow_trace_demotion": false,
                "allow_overlay_promotion": false,
                "allow_recompute_promotion": false
            },
            "provenance": policy_provenance_json()
        })
    }

    #[test]
    fn sequence_semantics_ref_defaults_to_unspecified_until_profiles_are_defined() {
        assert_eq!(
            SequenceSemanticsRef::default(),
            SequenceSemanticsRef::Unspecified
        );
    }

    #[test]
    fn sequence_semantics_ref_round_trips_through_serde() {
        let linear_state_expected = serde_json::json!({"kind": "LinearState"});
        let bounded_kv_expected = serde_json::json!({"kind": "BoundedKv"});

        assert_eq!(
            serde_json::to_value(SequenceSemanticsRef::LinearState).unwrap(),
            linear_state_expected
        );

        assert_eq!(
            serde_json::to_value(SequenceSemanticsRef::BoundedKv).unwrap(),
            bounded_kv_expected
        );

        let encoded = serde_json::to_string(&SequenceSemanticsRef::BoundedKv).unwrap();
        let decoded: SequenceSemanticsRef = serde_json::from_str(&encoded).unwrap();
        let linear_state_from_shape: SequenceSemanticsRef =
            serde_json::from_value(linear_state_expected).unwrap();
        let bounded_kv_from_shape: SequenceSemanticsRef =
            serde_json::from_value(bounded_kv_expected).unwrap();

        assert_eq!(decoded, SequenceSemanticsRef::BoundedKv);
        assert_eq!(linear_state_from_shape, SequenceSemanticsRef::LinearState);
        assert_eq!(bounded_kv_from_shape, SequenceSemanticsRef::BoundedKv);
    }

    #[test]
    fn compile_types_round_trip() {
        let request = request_fixture();
        let encoded = serde_json::to_string(&request).expect("request serializes");
        let decoded: CompileRequest = serde_json::from_str(&encoded).expect("request deserializes");
        assert_eq!(decoded, request);
        assert_eq!(
            serde_json::to_value(&request).expect("request serializes"),
            request_json()
        );

        let policy = policy_fixture();
        let encoded = serde_json::to_string(&policy).expect("policy serializes");
        let decoded: ResolvedCompilePolicy =
            serde_json::from_str(&encoded).expect("policy deserializes");
        assert_eq!(decoded, policy);
        assert_eq!(
            serde_json::to_value(&policy).expect("policy serializes"),
            policy_json()
        );
    }

    #[test]
    fn compile_profile_spec_includes_risk_policy_field() {
        let spec = CompileProfileSpec {
            schema_version: COMPILE_PROFILE_SPEC_VERSION.to_owned(),
            id: CompileProfileId::from("Default"),
            defaults_hash: Hash256::from_bytes([9; 32]),
            observability: ObservabilityMode::Invariant,
            trace_budget: TraceBudget {
                max_events_per_slice: 1,
                max_bytes_per_frame: 32,
                drop_policy: TraceDropPolicy::DropOldest,
            },
            range_caps: RangeCapsSpec::default_v2(),
            observation_caps: ObservationProfileCaps::default_v2(),
            repair_policy: RepairPolicy::for_profile(RepairPolicyProfile::Default),
            risk_policy: objective_fixture().risk,
            knob_defaults: CompileKnobPartialValues::default(),
            knob_bounds: CompileKnobPartialBounds {
                placement: Some(PlacementKnobBounds {
                    max_profile: PlacementProfile::PackedExperts,
                }),
                ..CompileKnobPartialBounds::default()
            },
            locks: KnobLockSet::default(),
        };

        let value = serde_json::to_value(&spec).expect("profile spec serializes");
        assert_eq!(
            value,
            serde_json::json!({
                "schema_version": "2.0.0",
                "id": "Default",
                "defaults_hash": hash_json(9),
                "observability": {"kind": "Invariant"},
                "trace_budget": {
                    "max_events_per_slice": 1,
                    "max_bytes_per_frame": 32,
                    "drop_policy": {"kind": "DropOldest"}
                },
                "range_caps": {
                    "profile_chunk_max": 256,
                    "profile_tile_max": 256,
                    "profile_tile_min": 16,
                    "renorm_strategy": {"kind": "ExactPostBoundaryOnly"}
                },
                "observation_caps": {
                    "required_max": null,
                    "important_max": 256,
                    "diagnostic_max": 256,
                    "best_effort_max": 256
                },
                "repair_policy": {
                    "max_refinement_iters": 4,
                    "allow_placement_profile_fallback": true,
                    "allow_trace_demotion": true,
                    "allow_overlay_promotion": true,
                    "allow_recompute_promotion": true
                },
                "risk_policy": risk_policy_json("Weak"),
                "knob_defaults": empty_partial_values_json(),
                "knob_bounds": {
                    "placement": {"max_profile": {"kind": "PackedExperts"}},
                    "observation": null,
                    "range": null,
                    "storage": null,
                    "sram": null,
                    "rom_window": null,
                    "overlay": null,
                    "schedule": null
                },
                "locks": {"locked": []}
            })
        );
        let decoded: CompileProfileSpec =
            serde_json::from_value(value).expect("profile spec deserializes");
        assert_eq!(decoded, spec);
    }

    #[test]
    fn compile_profile_spec_fixtures_round_trip() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");
        assert_eq!(BRINGUP_COMPILE_PROFILE_ID, "Bringup");
        assert_eq!(DEFAULT_COMPILE_PROFILE_ID, "Default");
        assert_eq!(TRACE_COMPILE_PROFILE_ID, "Trace");
        assert_eq!(RECOVERY_COMPILE_PROFILE_ID, "Recovery");
        assert_eq!(
            specs
                .iter()
                .map(|spec| spec.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                BRINGUP_COMPILE_PROFILE_ID,
                DEFAULT_COMPILE_PROFILE_ID,
                TRACE_COMPILE_PROFILE_ID,
                RECOVERY_COMPILE_PROFILE_ID
            ]
        );

        for (source, spec) in [
            (BRINGUP_COMPILE_PROFILE_TOML, &specs[0]),
            (DEFAULT_COMPILE_PROFILE_TOML, &specs[1]),
            (TRACE_COMPILE_PROFILE_TOML, &specs[2]),
            (RECOVERY_COMPILE_PROFILE_TOML, &specs[3]),
        ] {
            assert_eq!(spec.schema_version, COMPILE_PROFILE_SPEC_VERSION);
            assert!(
                !source.contains("relaxations"),
                "profile fixture must not expose profile-time relaxations"
            );
            assert_eq!(
                load_compile_profile_spec(source).expect("profile reparses"),
                *spec
            );
            assert_eq!(spec.observation_caps.required_max, None);
            assert!(
                spec.knob_defaults.placement.is_some()
                    && spec.knob_defaults.observation.is_some()
                    && spec.knob_defaults.range.is_some()
                    && spec.knob_defaults.storage.is_some()
                    && spec.knob_defaults.sram.is_some()
                    && spec.knob_defaults.rom_window.is_some()
                    && spec.knob_defaults.overlay.is_some()
                    && spec.knob_defaults.schedule.is_some()
            );
            assert!(
                spec.knob_bounds.placement.is_some()
                    && spec.knob_bounds.observation.is_some()
                    && spec.knob_bounds.range.is_some()
                    && spec.knob_bounds.storage.is_some()
                    && spec.knob_bounds.sram.is_some()
                    && spec.knob_bounds.rom_window.is_some()
                    && spec.knob_bounds.overlay.is_some()
                    && spec.knob_bounds.schedule.is_some()
            );
            assert_eq!(
                spec.risk_policy
                    .fallback_profile
                    .as_ref()
                    .map(|profile| profile.as_str()),
                Some(RECOVERY_COMPILE_PROFILE_ID),
                "{} must fall back to the canonical Recovery profile id",
                spec.id
            );
        }

        assert_eq!(
            specs[0].risk_policy.calibration_confidence_requirement,
            CalibrationConfidenceRequirement::NoMinimumConfidence
        );
        assert_eq!(
            specs[0].range_caps.renorm_strategy,
            RenormStrategyPolicy::ExactPostBoundaryOnly
        );
        assert_eq!(specs[0].observation_caps.important_max, 32);
        assert_eq!(
            specs[1].range_caps.renorm_strategy,
            RenormStrategyPolicy::DynamicMargin {
                margin_q16_16: 0x4000
            }
        );
        assert_eq!(specs[1].observation_caps.best_effort_max, 64);
        assert_eq!(specs[2].observation_caps.diagnostic_max, 1024);
        assert_eq!(specs[3].observation_caps.best_effort_max, 0);
        for spec in &specs[1..] {
            match spec.risk_policy.calibration_confidence_requirement {
                CalibrationConfidenceRequirement::NoMinimumConfidence => {
                    panic!("non-bringup profiles require measured confidence")
                }
                CalibrationConfidenceRequirement::AtLeast { class } => {
                    assert!(
                        class.rank() >= CalibrationConfidenceClass::Transferred.rank(),
                        "non-bringup profile {} requires {class:?}",
                        spec.id
                    );
                }
            }
        }
    }

    #[test]
    fn compile_profile_spec_v2_serde_round_trip() {
        for spec in canonical_compile_profile_specs().expect("canonical profiles parse") {
            let value = serde_json::to_value(&spec).expect("profile serializes");
            assert_eq!(value["schema_version"], COMPILE_PROFILE_SPEC_VERSION);
            assert_eq!(value["range_caps"]["profile_chunk_max"], 256);
            assert_eq!(value["range_caps"]["profile_tile_max"], 256);
            assert_eq!(value["range_caps"]["profile_tile_min"], 16);
            assert_eq!(
                value["observation_caps"]["required_max"],
                serde_json::Value::Null
            );

            let canonical_bytes = compile_profile_defaults_hash_canonical_json_bytes(&spec)
                .expect("profile has canonical JSON bytes");
            let reparsed: CompileProfileSpec =
                serde_json::from_slice(&canonical_bytes).expect("canonical JSON reparses");
            let recanonicalized = compile_profile_defaults_hash_canonical_json_bytes(&reparsed)
                .expect("reparsed profile has canonical JSON bytes");

            assert_eq!(
                recanonicalized, canonical_bytes,
                "canonical JSON is byte-stable for {}",
                spec.id
            );
        }
    }

    #[test]
    fn compile_profile_spec_v2_invariants_validated_on_load() {
        fn assert_invalid(source: String, invariant: &str) {
            match load_compile_profile_spec(&source).expect_err("profile invariant rejects") {
                CompileProfileSpecLoadError::InvalidInvariant {
                    invariant: actual, ..
                } => assert_eq!(actual, invariant),
                err => panic!("unexpected error: {err:?}"),
            }
        }

        let source = DEFAULT_COMPILE_PROFILE_TOML;
        assert_invalid(
            source.replace("profile_chunk_max = 256", "profile_chunk_max = 0"),
            "profile_chunk_max > 0",
        );
        assert_invalid(
            source.replace("profile_tile_max = 256", "profile_tile_max = 0"),
            "profile_tile_max > 0",
        );
        assert_invalid(
            source.replace("profile_tile_min = 16", "profile_tile_min = 0"),
            "profile_tile_min > 0",
        );
        assert_invalid(
            source
                .replace("profile_tile_max = 256", "profile_tile_max = 15")
                .replace("profile_tile_min = 16", "profile_tile_min = 16"),
            "profile_tile_min <= profile_tile_max",
        );
        assert_invalid(
            source.replace(
                "renorm_strategy = { kind = \"DynamicMargin\", margin_q16_16 = 16384 }",
                "renorm_strategy = { kind = \"DynamicMargin\", margin_q16_16 = 65536 }",
            ),
            "DynamicMargin.margin_q16_16 < 0x1_0000",
        );
    }

    #[test]
    fn compile_profile_spec_v2_loaded_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let capture = installed_trace_capture();
        let spec = load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML)
            .expect("v2 profile loads and emits telemetry");
        tracing::callsite::rebuild_interest_cache();
        let range_caps_hash = compile_profile_component_hash("RangeCapsSpec", &spec.range_caps)
            .expect("range caps hash computes")
            .to_hex();
        let observation_caps_hash =
            compile_profile_component_hash("ObservationProfileCaps", &spec.observation_caps)
                .expect("observation caps hash computes")
                .to_hex();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str) == Some(PROFILE_SPEC_V2_LOADED_EVENT)
                    && event.field("profile_id") == Some("Default")
            })
            .expect("profile v2 loaded event is captured");

        assert_eq!(event.field("profile_id"), Some("Default"));
        assert_eq!(
            event.field("schema_version"),
            Some(COMPILE_PROFILE_SPEC_VERSION)
        );
        assert_eq!(
            event.field("range_caps_hash"),
            Some(range_caps_hash.as_str())
        );
        assert_eq!(
            event.field("observation_caps_hash"),
            Some(observation_caps_hash.as_str())
        );
    }

    #[test]
    fn compile_profile_spec_v1_fixture_rejected_with_typed_error() {
        let v1_fixture = DEFAULT_COMPILE_PROFILE_TOML
            .lines()
            .filter(|line| {
                !line.starts_with("schema_version")
                    && !line.starts_with("[range_caps]")
                    && !line.starts_with("profile_chunk_max")
                    && !line.starts_with("profile_tile_max")
                    && !line.starts_with("profile_tile_min")
                    && !line.starts_with("renorm_strategy")
                    && !line.starts_with("[observation_caps]")
                    && !line.starts_with("important_max")
                    && !line.starts_with("diagnostic_max")
                    && !line.starts_with("best_effort_max")
            })
            .collect::<Vec<_>>()
            .join("\n");

        match load_compile_profile_spec(&v1_fixture).expect_err("v1 profile rejects") {
            CompileProfileSpecLoadError::UnsupportedSchemaVersion {
                code,
                actual,
                expected,
            } => {
                assert_eq!(code, COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE);
                assert_eq!(actual, "1.0.0");
                assert_eq!(expected, COMPILE_PROFILE_SPEC_VERSION);
            }
            err => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn compile_profile_spec_v1_rejected_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let v1_fixture = DEFAULT_COMPILE_PROFILE_TOML
            .replace(r#"schema_version = "2.0.0""#, r#"schema_version = "1.0.0""#);
        let capture = installed_trace_capture();
        let _ = load_compile_profile_spec(&v1_fixture);
        tracing::callsite::rebuild_interest_cache();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str)
                    == Some(PROFILE_SPEC_V1_REJECTED_EVENT)
                    && event.field("reason") == Some("unsupported_schema_version")
            })
            .expect("profile v1 rejected event is captured");

        assert_eq!(event.field("actual_version"), Some("1.0.0"));
        assert_eq!(event.field("fixture_path"), Some("inline"));
        assert_eq!(event.field("reason"), Some("unsupported_schema_version"));
    }

    #[test]
    fn compile_profile_spec_v2_invariant_failure_event_is_subscriber_captured() {
        let _guard = crate::TRACE_CAPTURE_LOCK
            .lock()
            .expect("trace capture lock is healthy");
        let capture = installed_trace_capture();
        let invalid =
            DEFAULT_COMPILE_PROFILE_TOML.replace("profile_tile_max = 256", "profile_tile_max = 0");
        let _ = load_compile_profile_spec(&invalid);
        tracing::callsite::rebuild_interest_cache();
        let events = capture.events.lock().expect("capture lock is healthy");
        let event = events
            .iter()
            .find(|event| {
                event.fields.get("event").map(String::as_str)
                    == Some(PROFILE_SPEC_V2_INVARIANT_FAILURE_EVENT)
                    && event.field("invariant") == Some("profile_tile_max > 0")
            })
            .expect("profile v2 invariant failure event is captured");

        assert_eq!(event.field("profile_id"), Some("Default"));
        assert_eq!(
            event.field("schema_version"),
            Some(COMPILE_PROFILE_SPEC_VERSION)
        );
        assert_eq!(event.field("invariant"), Some("profile_tile_max > 0"));
        assert_eq!(event.field("field"), Some("range_caps.profile_tile_max"));
        assert_eq!(event.field("value"), Some("0"));
    }

    #[test]
    fn compile_profile_spec_explicit_v1_schema_version_rejected_with_typed_error() {
        let v1_fixture = DEFAULT_COMPILE_PROFILE_TOML
            .replace(r#"schema_version = "2.0.0""#, r#"schema_version = "1.0.0""#);

        match load_compile_profile_spec(&v1_fixture).expect_err("v1 profile rejects") {
            CompileProfileSpecLoadError::UnsupportedSchemaVersion {
                code,
                actual,
                expected,
            } => {
                assert_eq!(code, COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE);
                assert_eq!(actual, "1.0.0");
                assert_eq!(expected, COMPILE_PROFILE_SPEC_VERSION);
            }
            err => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn compile_profile_spec_mismatched_schema_version_rejected_with_typed_error() {
        let future_fixture = DEFAULT_COMPILE_PROFILE_TOML
            .replace(r#"schema_version = "2.0.0""#, r#"schema_version = "3.0.0""#);

        match load_compile_profile_spec(&future_fixture).expect_err("future profile rejects") {
            CompileProfileSpecLoadError::UnsupportedSchemaVersion {
                code,
                actual,
                expected,
            } => {
                assert_eq!(code, COMPILE_PROFILE_SPEC_UNSUPPORTED_SCHEMA_CODE);
                assert_eq!(actual, "3.0.0");
                assert_eq!(expected, COMPILE_PROFILE_SPEC_VERSION);
            }
            err => panic!("unexpected error: {err:?}"),
        }
    }

    #[test]
    fn compile_profile_spec_v2_missing_schema_version_field_rejected() {
        let without_schema_version = DEFAULT_COMPILE_PROFILE_TOML
            .lines()
            .filter(|line| !line.starts_with("schema_version"))
            .collect::<Vec<_>>()
            .join("\n");

        assert!(matches!(
            load_compile_profile_spec(&without_schema_version),
            Err(CompileProfileSpecLoadError::MissingRequiredField {
                field: "schema_version"
            })
        ));
    }

    #[test]
    fn compile_profile_spec_v2_missing_range_caps_field_rejected() {
        let without_range_caps = DEFAULT_COMPILE_PROFILE_TOML
            .lines()
            .filter(|line| {
                !line.starts_with("[range_caps]")
                    && !line.starts_with("profile_chunk_max")
                    && !line.starts_with("profile_tile_max")
                    && !line.starts_with("profile_tile_min")
                    && !line.starts_with("renorm_strategy")
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(matches!(
            load_compile_profile_spec(&without_range_caps),
            Err(CompileProfileSpecLoadError::MissingRequiredField {
                field: "range_caps"
            })
        ));
    }

    #[test]
    fn compile_profile_spec_v2_missing_observation_caps_field_rejected() {
        let without_observation_caps = DEFAULT_COMPILE_PROFILE_TOML
            .lines()
            .filter(|line| {
                !line.starts_with("[observation_caps]")
                    && !line.starts_with("important_max")
                    && !line.starts_with("diagnostic_max")
                    && !line.starts_with("best_effort_max")
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(matches!(
            load_compile_profile_spec(&without_observation_caps),
            Err(CompileProfileSpecLoadError::MissingRequiredField {
                field: "observation_caps"
            })
        ));
    }

    #[test]
    fn observation_profile_caps_required_max_none_is_default_v2() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");

        assert!(
            specs
                .iter()
                .all(|spec| spec.observation_caps.required_max.is_none())
        );
    }

    #[test]
    fn range_caps_renorm_strategy_policy_round_trips_both_variants() {
        for renorm_strategy in [
            RenormStrategyPolicy::ExactPostBoundaryOnly,
            RenormStrategyPolicy::DynamicMargin {
                margin_q16_16: 0x4000,
            },
        ] {
            let caps = RangeCapsSpec {
                renorm_strategy,
                ..RangeCapsSpec::default_v2()
            };
            let encoded = serde_json::to_string(&caps).expect("range caps serialize");
            let decoded: RangeCapsSpec =
                serde_json::from_str(&encoded).expect("range caps deserialize");

            assert_eq!(decoded, caps);
        }
    }

    #[test]
    fn compile_profile_spec_defaults_hash_is_deterministic() {
        let specs = canonical_compile_profile_specs().expect("canonical profiles parse");

        for spec in specs {
            let hash = compile_profile_defaults_hash(&spec).expect("hashable profile spec");
            assert_eq!(
                spec.defaults_hash, hash,
                "defaults_hash mismatch for {}",
                spec.id
            );
            assert_eq!(
                hash,
                compile_profile_defaults_hash(&spec).expect("hashable profile spec"),
                "defaults_hash is deterministic for {}",
                spec.id
            );
        }
    }

    #[test]
    fn compile_profile_spec_defaults_hash_uses_canonical_domain_preimage() {
        assert_eq!(
            COMPILE_PROFILE_DEFAULTS_HASH_DOMAIN,
            b"gbf:gbf-policy:CompileProfileSpec:compile_profile_spec:2.0.0\0"
        );

        let spec = load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML)
            .expect("default profile parses");
        let canonical_bytes = compile_profile_defaults_hash_canonical_json_bytes(&spec)
            .expect("profile has canonical JSON bytes");
        let canonical_json =
            std::str::from_utf8(&canonical_bytes).expect("canonical JSON is UTF-8");

        assert!(
            canonical_json.starts_with(r#"{"defaults_hash":"sha256:0000000000000000000000000000000000000000000000000000000000000000","id":"Default""#),
            "top-level canonical JSON keys must be lexicographically ordered"
        );
        assert!(
            canonical_json.contains(r#""risk_policy":{"calibration_confidence_requirement":"#),
            "nested object keys must be lexicographically ordered"
        );

        let mut struct_order = spec.clone();
        struct_order.defaults_hash = Hash256::ZERO;
        let plain_serde_bytes =
            serde_json::to_vec(&struct_order).expect("plain profile spec serializes");
        assert_ne!(
            canonical_bytes, plain_serde_bytes,
            "canonical JSON preimage must not be serde_json's struct field order"
        );

        let hash = compile_profile_defaults_hash(&spec).expect("profile hash computes");
        let no_domain_digest = Sha256::digest(&canonical_bytes);
        assert_ne!(
            hash,
            Hash256::from_bytes(no_domain_digest.into()),
            "defaults_hash must include the explicit CompileProfileSpec domain separator"
        );
    }

    #[test]
    fn compile_knobs_monotone_order_per_subknob() {
        assert!(
            PlacementProfile::PackedExperts.is_monotone_successor_of(&PlacementProfile::Budgeted)
        );
        assert!(
            ProbeCollectionLevel::Verbose
                .is_monotone_successor_of(&ProbeCollectionLevel::Operational)
        );
        assert!(
            ReductionPlanCeiling::Adaptive
                .is_monotone_successor_of(&ReductionPlanCeiling::Conservative)
        );
        assert!(
            StorageMaterialization::SpillColdValues
                .is_monotone_successor_of(&StorageMaterialization::RecomputePureValues)
        );
        assert!(
            SramPageAggression::MinimizeResident
                .is_monotone_successor_of(&SramPageAggression::PackCold)
        );
        assert!(
            RomKernelResidencyBias::PreferWramOverlay
                .is_monotone_successor_of(&RomKernelResidencyBias::PreferExpertBank)
        );
        assert!(
            RomKernelDuplicationBias::DuplicateAllFit
                .is_monotone_successor_of(&RomKernelDuplicationBias::DuplicateHot)
        );
        assert!(
            OverlayPromotion::EligibleKernels.is_monotone_successor_of(&OverlayPromotion::TinyLuts)
        );
        assert!(
            ScheduleTileSearch::ProfileGuided.is_monotone_successor_of(&ScheduleTileSearch::Local)
        );
        assert!(
            ScheduleSliceCoarsening::Coarse
                .is_monotone_successor_of(&ScheduleSliceCoarsening::Balanced)
        );
        assert!(
            ScheduleResourcePressure::FitFirst
                .is_monotone_successor_of(&ScheduleResourcePressure::Balanced)
        );

        assert!(
            PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::PackedExperts,
            })
        );
        assert!(
            PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
        assert!(
            !PlacementKnobBounds {
                max_profile: PlacementProfile::PackedExperts,
            }
            .is_monotone_successor_of(&PlacementKnobBounds {
                max_profile: PlacementProfile::Budgeted,
            })
        );
        assert!(
            !PlacementProfile::StrictOnePerBank
                .is_monotone_successor_of(&PlacementProfile::Budgeted)
        );
        assert!(
            !ObservationKnobBounds {
                max_trace_demotion: TraceDemotionLevel::RequiredOnly,
                max_probe_level: ProbeCollectionLevel::Verbose,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
                max_trace_demotion: TraceDemotionLevel::RequiredOnly,
                max_probe_level: ProbeCollectionLevel::Operational,
            })
        );
        assert!(
            ObservationKnobBounds {
                max_trace_demotion: TraceDemotionLevel::RequiredOnly,
                max_probe_level: ProbeCollectionLevel::Operational,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
                max_trace_demotion: TraceDemotionLevel::RequiredOnly,
                max_probe_level: ProbeCollectionLevel::Operational,
            })
        );
        assert!(
            !RangeKnobBounds {
                max_reduction_ceiling: ReductionPlanCeiling::Adaptive,
            }
            .is_monotone_successor_of(&RangeKnobBounds {
                max_reduction_ceiling: ReductionPlanCeiling::Conservative,
            })
        );
        assert!(
            !StorageKnobBounds {
                max_materialization: StorageMaterialization::SpillColdValues,
            }
            .is_monotone_successor_of(&StorageKnobBounds {
                max_materialization: StorageMaterialization::RecomputePureValues,
            })
        );
        assert!(
            !SramKnobBounds {
                max_page_aggression: SramPageAggression::MinimizeResident,
                max_spill_policy: SramSpillPolicy::SpillOnPressure,
            }
            .is_monotone_successor_of(&SramKnobBounds {
                max_page_aggression: SramPageAggression::PackCold,
                max_spill_policy: SramSpillPolicy::SpillOnPressure,
            })
        );
        assert!(
            !RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferWramOverlay,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            }
            .is_monotone_successor_of(&RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            })
        );
        assert!(
            !RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateAllFit,
            }
            .is_monotone_successor_of(&RomWindowKnobBounds {
                max_kernel_residency_bias: RomKernelResidencyBias::PreferExpertBank,
                max_kernel_duplication_bias: RomKernelDuplicationBias::DuplicateHot,
            })
        );
        assert!(
            !OverlayKnobBounds {
                max_promotion: OverlayPromotion::EligibleKernels,
            }
            .is_monotone_successor_of(&OverlayKnobBounds {
                max_promotion: OverlayPromotion::TinyLuts,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::ProfileGuided,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::FitFirst,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
                max_pressure_thresholds: default_max_resource_pressure_thresholds(),
                max_stage_iteration_ceilings: default_max_stage_iteration_ceilings(),
            })
        );
    }

    #[test]
    fn policy_source_repair_proposal_variant_round_trips_but_is_not_default() {
        let source = PolicySource::RepairProposal {
            id: RepairProposalId("future-rp-1".to_owned()),
        };
        let expected = serde_json::json!({
            "kind": "RepairProposal",
            "id": "future-rp-1"
        });

        assert_eq!(
            serde_json::to_value(&source).expect("source serializes"),
            expected
        );

        let encoded = serde_json::to_string(&source).expect("source serializes");
        let decoded: PolicySource = serde_json::from_str(&encoded).expect("source deserializes");
        let decoded_from_shape: PolicySource =
            serde_json::from_value(expected).expect("source deserializes from public shape");

        assert_eq!(decoded, source);
        assert_eq!(decoded_from_shape, source);
        assert!(
            !compile_knobs_fixture()
                .provenance
                .iter()
                .flat_map(|entry| entry.chain.iter())
                .any(|provenance| matches!(provenance.source, PolicySource::RepairProposal { .. }))
        );
    }

    #[test]
    fn constraint_operation_has_no_authorized_relaxation_variant() {
        let value = serde_json::json!({"kind": "AuthorizedRelaxation"});

        assert!(serde_json::from_value::<ConstraintOperation>(value).is_err());
    }

    #[test]
    fn compile_request_rejects_unknown_field() {
        let mut value = serde_json::to_value(request_fixture()).expect("request serializes");
        value["bringup_relaxation"] = serde_json::json!(true);

        assert!(serde_json::from_value::<CompileRequest>(value).is_err());
    }

    #[test]
    fn canonical_default_bounds_fixture_round_trips() {
        let bounds = canonical_default_bounds_fixture();
        let encoded = serde_json::to_string(&bounds).expect("bounds serializes");
        let decoded: CompileKnobBounds =
            serde_json::from_str(&encoded).expect("bounds deserializes");

        assert_eq!(decoded, bounds);
        assert_eq!(
            serde_json::to_value(bounds).expect("bounds serialize"),
            default_bounds_json()
        );
    }

    #[test]
    fn compile_knob_paths_sort_by_knob_selector_then_field() {
        let paths = BTreeSet::from([
            CompileKnobPath {
                knob: CompileKnobId::Schedule,
                selector: None,
                field: None,
            },
            CompileKnobPath {
                knob: CompileKnobId::Placement,
                selector: Some(SelectorPath("expert.1".to_owned())),
                field: Some(FieldPath::from("profile")),
            },
        ]);

        let first = paths.into_iter().next().expect("path exists");
        assert_eq!(first.knob, CompileKnobId::Placement);
    }

    #[test]
    fn field_path_reexports_foundation_type_and_serializes_as_string() {
        fn accepts_foundation_field_path(_: gbf_foundation::FieldPath) {}

        let field = FieldPath::from("profile");
        accepts_foundation_field_path(field.clone());

        let path = CompileKnobPath {
            knob: CompileKnobId::Placement,
            selector: None,
            field: Some(field),
        };

        assert_eq!(
            serde_json::to_value(path).expect("path serializes"),
            serde_json::json!({
                "knob": {"kind": "Placement"},
                "selector": null,
                "field": "profile"
            })
        );
    }

    #[test]
    fn constraint_value_json_shapes_are_pinned() {
        let cases = [
            (
                ConstraintValue::PlacementProfile {
                    value: PlacementProfile::Budgeted,
                },
                serde_json::json!({
                    "kind": "PlacementProfile",
                    "value": {"kind": "Budgeted"}
                }),
            ),
            (
                ConstraintValue::ObservabilityMode {
                    value: ObservabilityMode::Flexible,
                },
                serde_json::json!({
                    "kind": "ObservabilityMode",
                    "value": {"kind": "Flexible"}
                }),
            ),
            (
                ConstraintValue::U16 { value: 512 },
                serde_json::json!({
                    "kind": "U16",
                    "value": 512
                }),
            ),
            (
                ConstraintValue::U32 { value: 70_000 },
                serde_json::json!({
                    "kind": "U32",
                    "value": 70_000
                }),
            ),
            (
                ConstraintValue::Bool { value: true },
                serde_json::json!({
                    "kind": "Bool",
                    "value": true
                }),
            ),
            (
                ConstraintValue::Text {
                    value: "bank0.kernel".to_owned(),
                },
                serde_json::json!({
                    "kind": "Text",
                    "value": "bank0.kernel"
                }),
            ),
        ];

        for (value, expected_json) in cases {
            assert_eq!(
                serde_json::to_value(&value).expect("value serializes"),
                expected_json
            );
            let decoded: ConstraintValue =
                serde_json::from_value(expected_json).expect("value deserializes");
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn policy_provenance_round_trips_with_optional_hashes() {
        let provenance = policy_fixture().provenance;
        let encoded = serde_json::to_string(&provenance).expect("provenance serializes");
        let decoded: PolicyProvenance =
            serde_json::from_str(&encoded).expect("provenance deserializes");

        assert_eq!(decoded, provenance);
        assert_eq!(
            serde_json::to_value(&provenance).expect("provenance serializes"),
            policy_provenance_json()
        );
    }

    #[test]
    fn compile_knobs_round_trip() {
        let knobs = compile_knobs_fixture();
        let encoded = serde_json::to_string(&knobs).expect("knobs serializes");
        let decoded: CompileKnobs = serde_json::from_str(&encoded).expect("knobs deserializes");

        assert_eq!(decoded, knobs);
        assert_eq!(
            serde_json::to_value(&knobs).expect("knobs serializes"),
            knobs_json()
        );
    }

    #[test]
    fn compile_request_required_features_are_sorted() {
        let encoded = serde_json::to_string(&request_fixture()).expect("request serializes");
        let artifact = encoded
            .find("ArtifactValidation")
            .expect("artifact feature present");
        let policy = encoded
            .find("PolicyResolution")
            .expect("policy feature present");

        assert!(artifact < policy);
    }

    #[test]
    fn effective_constraints_round_trip_without_runtime_budget() {
        let constraints = policy_fixture().effective_constraints;
        let encoded = serde_json::to_string(&constraints).expect("constraints serializes");
        let decoded: EffectiveConstraints =
            serde_json::from_str(&encoded).expect("constraints deserializes");

        assert_eq!(decoded, constraints);
    }

    mod knobs {
        use super::*;

        #[test]
        fn serde_round_trip() {
            let knobs = compile_knobs_fixture();
            let encoded = serde_json::to_string(&knobs).expect("knobs serialize");
            let decoded: CompileKnobs = serde_json::from_str(&encoded).expect("knobs deserialize");

            assert_eq!(decoded, knobs);
        }

        #[test]
        fn monotone_orders() {
            assert_eq!(PlacementProfile::StrictOnePerBank.rank(), 0);
            assert_eq!(PlacementProfile::Budgeted.rank(), 1);
            assert_eq!(PlacementProfile::PackedExperts.rank(), 2);
            assert!(
                PlacementProfile::Budgeted
                    .is_monotone_successor_of(&PlacementProfile::StrictOnePerBank)
            );
            assert!(
                StorageMaterialization::SpillColdValues
                    .is_monotone_successor_of(&StorageMaterialization::RecomputePureValues)
            );
            assert!(
                RomKernelDuplicationBias::DuplicateAllFit
                    .is_monotone_successor_of(&RomKernelDuplicationBias::DuplicateHot)
            );
            assert!(
                ScheduleSliceCoarsening::Coarse
                    .is_monotone_successor_of(&ScheduleSliceCoarsening::Balanced)
            );
        }

        #[test]
        fn knob_id_exhaustive() {
            let ids = f_b16_refinement_knob_ids();
            assert_eq!(ids.len(), 16);
            assert!(ids.contains(&CompileKnobId::PlacementProfile));
            assert!(ids.contains(&CompileKnobId::ObservationTraceDemotion));
            assert!(ids.contains(&CompileKnobId::StorageMaterializationOverrides));
            assert!(ids.contains(&CompileKnobId::RomKernelResidencyOverrides));
            assert!(ids.contains(&CompileKnobId::StageIterationCeilings));
        }
    }

    mod selectors {
        use super::*;

        #[test]
        fn id_exhaustive() {
            let classes = [
                SliceClass::Micro,
                SliceClass::Frame,
                SliceClass::TokenBoundary,
                SliceClass::TraceHeavy,
            ];
            let encoded = serde_json::to_string(&TileSelector::SliceClass {
                class: SliceClass::TraceHeavy,
            })
            .expect("selector serializes");
            let decoded: TileSelector =
                serde_json::from_str(&encoded).expect("selector deserializes");

            assert_eq!(classes.len(), 4);
            assert_eq!(
                decoded,
                TileSelector::SliceClass {
                    class: SliceClass::TraceHeavy
                }
            );
        }
    }

    mod overrides {
        use super::*;

        #[test]
        fn serde_round_trip() {
            let overrides = CompileKnobOverrides {
                disabled_optional_probes: BTreeSet::from([TraceProbeId(7)]),
                forced_kernel_residency: BTreeMap::from([(
                    KernelSelector::KernelSpec {
                        id: KernelSpecId(3),
                    },
                    KernelResidency::WramOverlay,
                )]),
                forced_recompute: BTreeSet::from([ValueSelector::Value { id: ValueId(9) }]),
                reduction_ceiling_overrides: BTreeMap::from([(
                    ReductionSelector::Layer { id: LayerId(1) },
                    ReductionPlanCeiling::Adaptive,
                )]),
                tile_class_overrides: BTreeMap::from([(
                    TileSelector::SliceClass {
                        class: SliceClass::Frame,
                    },
                    BTreeSet::from([TileCandidateClass::Balanced]),
                )]),
                ..CompileKnobOverrides::default()
            };

            let encoded = serde_json::to_string(&overrides).expect("overrides serialize");
            let decoded: CompileKnobOverrides =
                serde_json::from_str(&encoded).expect("overrides deserialize");

            assert_eq!(decoded, overrides);
        }

        #[test]
        fn monotone_insert() {
            let mut previous = CompileKnobOverrides::default();
            let mut next = CompileKnobOverrides::default();
            let selector = KernelSelector::KernelSpec {
                id: KernelSpecId(1),
            };
            next.forced_kernel_residency
                .insert(selector, KernelResidency::WramOverlay);

            assert!(overrides_are_monotone(&previous, &next));

            previous.forced_kernel_residency.insert(
                KernelSelector::Section { id: SectionId(2) },
                KernelResidency::CoResident,
            );
            next.forced_kernel_residency.insert(
                KernelSelector::Section { id: SectionId(2) },
                KernelResidency::WramOverlay,
            );
            assert!(overrides_are_monotone(&previous, &next));
        }

        #[test]
        fn monotone_no_delete() {
            let selector = ValueSelector::Value { id: ValueId(1) };
            let previous = CompileKnobOverrides {
                forced_recompute: BTreeSet::from([selector]),
                ..CompileKnobOverrides::default()
            };
            let next = CompileKnobOverrides::default();

            assert!(!overrides_are_monotone(&previous, &next));
        }

        fn overrides_are_monotone(
            previous: &CompileKnobOverrides,
            next: &CompileKnobOverrides,
        ) -> bool {
            previous
                .disabled_optional_probes
                .is_subset(&next.disabled_optional_probes)
                && previous.forced_recompute.is_subset(&next.forced_recompute)
                && previous
                    .forced_kernel_residency
                    .iter()
                    .all(|(selector, old)| {
                        next.forced_kernel_residency
                            .get(selector)
                            .is_some_and(|new| new.rank() >= old.rank())
                    })
                && previous
                    .reduction_ceiling_overrides
                    .iter()
                    .all(|(selector, old)| {
                        next.reduction_ceiling_overrides
                            .get(selector)
                            .is_some_and(|new| new.rank() >= old.rank())
                    })
                && previous.tile_class_overrides.iter().all(|(selector, old)| {
                    next.tile_class_overrides
                        .get(selector)
                        .is_some_and(|new| new.is_subset(old))
                })
        }
    }

    mod delta {
        use super::*;

        #[test]
        fn serde_round_trip() {
            let delta = ConstraintDelta {
                changes: vec![KnobDelta::UpdatePressureThreshold {
                    update: ResourcePressureUpdate::SliceCycles {
                        limit: PressureLimit {
                            soft: 1200,
                            hard: 1400,
                        },
                    },
                }],
            };

            let encoded = serde_json::to_string(&delta).expect("delta serializes");
            let decoded: ConstraintDelta =
                serde_json::from_str(&encoded).expect("delta deserializes");

            assert_eq!(decoded, delta);
        }

        #[test]
        fn admissibility_locked() {
            let mut knobs = compile_knobs_fixture();
            knobs.locks.locked.insert(CompileKnobId::PlacementProfile);
            let rejection = check_delta_admissible(
                &KnobDelta::AdvancePlacementProfile {
                    to: PlacementProfile::PackedExperts,
                },
                &knobs,
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("locked knob rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::KnobLocked {
                    knob: CompileKnobId::PlacementProfile
                }
            ));
        }

        #[test]
        fn admissibility_policy_toggle() {
            let rejection = check_delta_admissible(
                &KnobDelta::PromoteOverlay {
                    to: OverlayPromotion::EligibleKernels,
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
                ObservabilityMode::Flexible,
            )
            .expect_err("disabled overlay promotion rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::PolicyToggleDisabled { toggle, .. }
                    if toggle == "allow_overlay_promotion"
            ));
        }

        #[test]
        fn admissibility_bounds() {
            let mut knobs = compile_knobs_fixture();
            knobs.bounds.placement.max_profile = PlacementProfile::Budgeted;
            let rejection = check_delta_admissible(
                &KnobDelta::AdvancePlacementProfile {
                    to: PlacementProfile::PackedExperts,
                },
                &knobs,
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("beyond max rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::BeyondBounds {
                    knob: CompileKnobId::PlacementProfile,
                    ..
                }
            ));
        }

        #[test]
        fn admissibility_monotone() {
            let rejection = check_delta_admissible(
                &KnobDelta::AdvancePlacementProfile {
                    to: PlacementProfile::StrictOnePerBank,
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("backward step rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::NotMonotone {
                    knob: CompileKnobId::PlacementProfile,
                    ..
                }
            ));
        }

        #[test]
        fn invariant_blocks_demotion() {
            let rejection = check_delta_admissible(
                &KnobDelta::SetTraceDemotion {
                    to: TraceDemotionLevel::DropBestEffort,
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Invariant,
            )
            .expect_err("invariant observability rejects demotion");

            assert!(matches!(
                rejection,
                DeltaRejection::InvariantObservabilityViolation {
                    knob: CompileKnobId::ObservationTraceDemotion
                }
            ));
        }

        #[test]
        fn effectful_recompute_rejected() {
            let value = ValueSelector::Value { id: ValueId(5) };
            let facts = RecomputePurityFacts {
                effectful_values: BTreeSet::from([value.clone()]),
                ..RecomputePurityFacts::default()
            };
            let rejection = check_delta_admissible_with_recompute_purity(
                &KnobDelta::ForceRecompute {
                    values: BTreeSet::from([value.clone()]),
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
                &facts,
            )
            .expect_err("effectful recompute rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::EffectfulRecompute { value: rejected }
                    if rejected == value
            ));
        }

        #[test]
        fn pure_recompute_allowed() {
            let value = ValueSelector::Value { id: ValueId(6) };
            let facts = RecomputePurityFacts {
                pure_values: BTreeSet::from([value.clone()]),
                ..RecomputePurityFacts::default()
            };

            check_delta_admissible_with_recompute_purity(
                &KnobDelta::ForceRecompute {
                    values: BTreeSet::from([value]),
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
                &facts,
            )
            .expect("pure force recompute is admissible");
        }

        #[test]
        fn sram_spill_policy_delta_is_backed_by_value_and_bounds() {
            check_delta_admissible(
                &KnobDelta::AdvanceSramSpillPolicy {
                    to: SramSpillPolicy::SpillEager,
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect("spill policy advances against explicit sram field");

            let mut knobs = compile_knobs_fixture();
            knobs.bounds.sram.max_spill_policy = SramSpillPolicy::NoSpill;
            let rejection = check_delta_admissible(
                &KnobDelta::AdvanceSramSpillPolicy {
                    to: SramSpillPolicy::SpillEager,
                },
                &knobs,
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("spill policy max bound rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::BeyondBounds {
                    knob: CompileKnobId::SramSpillPolicy,
                    ..
                }
            ));
        }

        #[test]
        fn force_recompute_without_purity_facts_rejects_unknown() {
            let value = ValueSelector::Value { id: ValueId(7) };
            let rejection = check_delta_admissible(
                &KnobDelta::ForceRecompute {
                    values: BTreeSet::from([value.clone()]),
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("unknown recompute purity rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::EffectfulRecompute { value: rejected }
                    if rejected == value
            ));
        }

        #[test]
        fn resource_pressure_update_variants_exhaustive() {
            let byte = PressureLimit {
                soft: 10_u64,
                hard: 20_u64,
            };
            let cycles = PressureLimit {
                soft: 30_u64,
                hard: 40_u64,
            };
            let per_frame = PressureLimit {
                soft: 3_u16,
                hard: 4_u16,
            };
            let tiny = PressureLimit {
                soft: 1_u8,
                hard: 2_u8,
            };
            let updates = [
                ResourcePressureUpdate::WramHot { limit: byte },
                ResourcePressureUpdate::HramHot { limit: byte },
                ResourcePressureUpdate::Bank0Rom { limit: byte },
                ResourcePressureUpdate::SwitchableRomWindow { limit: byte },
                ResourcePressureUpdate::SramWindow { limit: byte },
                ResourcePressureUpdate::SliceCycles { limit: cycles },
                ResourcePressureUpdate::InterruptLatency { limit: cycles },
                ResourcePressureUpdate::TraceBytesPerFrame { limit: per_frame },
                ResourcePressureUpdate::PersistBytesPerFrame { limit: per_frame },
                ResourcePressureUpdate::OverlayInstallsPerFrame { limit: tiny },
                ResourcePressureUpdate::BankSwitchesPerToken { limit: per_frame },
                ResourcePressureUpdate::SramPageSwitchesPerToken { limit: per_frame },
            ];

            assert_eq!(updates.len(), 12);
        }
    }

    mod profile_defaults {
        use super::*;

        #[test]
        fn bringup() {
            assert_eq!(
                RepairPolicy::for_profile(RepairPolicyProfile::Bringup),
                RepairPolicy {
                    max_refinement_iters: 1,
                    allow_placement_profile_fallback: false,
                    allow_trace_demotion: false,
                    allow_overlay_promotion: false,
                    allow_recompute_promotion: false,
                }
            );
            let locks = f_b16_profile_lock_set(RepairPolicyProfile::Bringup);
            assert!(!locks.is_locked(CompileKnobId::ScheduleTileSearch));
            assert!(!locks.is_locked(CompileKnobId::ScheduleSliceCoarsening));
            assert!(locks.is_locked(CompileKnobId::PlacementProfile));
        }

        #[test]
        fn bringup_first_fit_strict_override() {
            assert_eq!(
                RepairPolicy::for_profile(RepairPolicyProfile::BringupFirstFit),
                RepairPolicy::bringup_strict_first_fit()
            );
            assert_eq!(
                RepairPolicy::for_profile(RepairPolicyProfile::BringupFirstFit)
                    .max_refinement_iters,
                0
            );
            assert_eq!(
                f_b16_profile_lock_set(RepairPolicyProfile::BringupFirstFit),
                f_b16_profile_lock_set(RepairPolicyProfile::Bringup)
            );
        }

        #[test]
        fn default() {
            assert_eq!(
                RepairPolicy::for_profile(RepairPolicyProfile::Default).max_refinement_iters,
                4
            );
            let locks = f_b16_profile_lock_set(RepairPolicyProfile::Default);
            assert!(locks.is_locked(CompileKnobId::ScheduleResourcePressure));
            assert!(locks.is_locked(CompileKnobId::StageIterationCeilings));
            assert!(!locks.is_locked(CompileKnobId::PlacementProfile));
        }

        #[test]
        fn default_unlocks_every_refinement_knob_except_pressure_and_stage_iters() {
            let locks = f_b16_profile_lock_set(RepairPolicyProfile::Default);
            let expected_locked = BTreeSet::from([
                CompileKnobId::ScheduleResourcePressure,
                CompileKnobId::StageIterationCeilings,
            ]);

            assert_eq!(locks.locked, expected_locked);
            for knob in f_b16_refinement_knob_ids() {
                let should_be_locked = expected_locked.contains(&knob);
                assert_eq!(
                    locks.is_locked(knob),
                    should_be_locked,
                    "{knob:?} lock state should match Default profile contract"
                );
            }
        }

        #[test]
        fn trace_invariant_locks_everything() {
            let locks = f_b16_profile_lock_set(RepairPolicyProfile::TraceInvariant);
            assert!(
                f_b16_refinement_knob_ids()
                    .into_iter()
                    .all(|knob| locks.is_locked(knob))
            );
        }

        #[test]
        fn recovery() {
            let policy = RepairPolicy::for_profile(RepairPolicyProfile::Recovery);
            assert_eq!(policy.max_refinement_iters, 6);
            assert!(policy.allow_placement_profile_fallback);
            assert!(policy.allow_trace_demotion);
            assert!(policy.allow_overlay_promotion);
            assert!(policy.allow_recompute_promotion);
        }

        #[test]
        fn placement_provenance_is_populated() {
            let profile =
                load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile loads");
            let knobs =
                resolve_initial_knobs_from_profile_spec(&profile).expect("initial knobs resolve");
            let entry = knobs
                .provenance
                .iter()
                .find(|entry| entry.path.knob == CompileKnobId::Placement)
                .expect("fixture has knob provenance");

            assert!(!entry.chain.is_empty());
            assert_eq!(
                entry.path.field.as_ref().map(FieldPath::as_str),
                Some("profile_default.placement")
            );
            assert!(
                entry
                    .chain
                    .iter()
                    .all(|provenance| !provenance.evidence.is_empty())
            );
        }

        #[test]
        fn resolves_initial_knobs_from_canonical_profiles() {
            let profiles = canonical_compile_profile_specs().expect("canonical profiles load");

            for profile in &profiles {
                let knobs = resolve_initial_knobs_from_profile_spec(profile)
                    .expect("initial knobs resolve from complete profile spec");
                let repair = resolve_repair_policy_from_profile_spec(profile)
                    .expect("repair policy resolves from profile id");

                assert_eq!(repair, profile.repair_policy);
                assert_eq!(
                    knobs.global.placement,
                    profile.knob_defaults.placement.expect("placement default")
                );
                assert_eq!(
                    knobs.bounds.placement,
                    profile.knob_bounds.placement.expect("placement bound")
                );
                assert_eq!(knobs.provenance.len(), 8);
                assert!(knobs.provenance.iter().all(|entry| {
                    entry
                        .chain
                        .iter()
                        .all(|provenance| matches!(provenance.source, PolicySource::ProfileDefault))
                }));
            }
        }

        #[test]
        fn resolve_initial_knobs_rejects_missing_profile_default() {
            let mut profile =
                load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile loads");
            profile.knob_defaults.placement = None;

            let err = resolve_initial_knobs_from_profile_spec(&profile)
                .expect_err("missing default rejects");

            assert!(matches!(
                err,
                InitialKnobsResolveError::MissingDefault {
                    field: "placement",
                    ..
                }
            ));
        }

        #[test]
        fn pressure_thresholds_have_provenance() {
            let profile =
                load_compile_profile_spec(DEFAULT_COMPILE_PROFILE_TOML).expect("profile loads");
            let objective = objective_fixture();
            let trace_budget = TraceBudget {
                max_events_per_slice: 11,
                max_bytes_per_frame: 300,
                drop_policy: TraceDropPolicy::DropOldest,
            };
            let chrome_budget = RuntimeChromeBudget {
                target: TargetProfileId::from("dmg-mbc5"),
                profile: CompileProfileId::from("Default"),
                runtime_nucleus_hash: Hash256::from_bytes([0x44; 32]),
                rom_slots: vec![
                    crate::budget::RomBudgetSlot {
                        id: gbf_foundation::BudgetSlotId::new(1),
                        class: crate::budget::BudgetSlotClass::Bank0Free,
                        usable_bytes: 4096,
                        reserved_slack: 96,
                        placement_caps: BTreeSet::from([PlacementProfile::Budgeted]),
                    },
                    crate::budget::RomBudgetSlot {
                        id: gbf_foundation::BudgetSlotId::new(2),
                        class: crate::budget::BudgetSlotClass::ExpertBank,
                        usable_bytes: 16_384,
                        reserved_slack: 384,
                        placement_caps: BTreeSet::from([PlacementProfile::PackedExperts]),
                    },
                ],
                memory_caps: crate::budget::RuntimeMemoryCapSection {
                    wram_usable_bytes: 8192,
                    sram_usable_bytes: 32 * 1024,
                    hram_usable_bytes: 127,
                    source_target_profile_hash: Hash256::from_bytes([0x45; 32]),
                },
                wram_reserved: 512,
                sram_reserved: 1024,
            };

            let resolved = resolve_resource_pressure_thresholds(
                &profile,
                Some(&chrome_budget),
                &objective,
                &trace_budget,
            )
            .expect("thresholds resolve");

            assert_eq!(resolved.thresholds.wram_hot.hard, 7680);
            assert_eq!(resolved.thresholds.bank0_rom.hard, 4000);
            assert_eq!(resolved.thresholds.switchable_rom_window.hard, 16_000);
            assert_eq!(resolved.thresholds.sram_window.hard, 31 * 1024);
            assert_eq!(resolved.thresholds.slice_cycles.hard, 8000);
            assert_eq!(resolved.thresholds.interrupt_latency.hard, 1000);
            assert_eq!(resolved.thresholds.trace_bytes_per_frame.hard, 300);
            assert_eq!(resolved.thresholds.bank_switches_per_token.hard, 5);
            assert_eq!(resolved.thresholds.sram_page_switches_per_token.hard, 1);
            assert_eq!(resolved.provenance.len(), 12);

            let wram = resolved
                .provenance
                .iter()
                .find(|entry| {
                    entry
                        .path
                        .field
                        .as_ref()
                        .is_some_and(|field| field.as_str() == "thresholds.wram_hot")
                })
                .expect("wram provenance exists");
            assert!(
                wram.chain
                    .iter()
                    .any(|provenance| matches!(provenance.source, PolicySource::TargetDefault))
            );

            let switches =
                resolved
                    .provenance
                    .iter()
                    .find(|entry| {
                        entry.path.field.as_ref().is_some_and(|field| {
                            field.as_str() == "thresholds.bank_switches_per_token"
                        })
                    })
                    .expect("switch provenance exists");
            assert!(switches.chain.iter().any(|provenance| {
                matches!(provenance.source, PolicySource::CompileRequestOverride)
            }));
        }

        #[test]
        fn pressure_update_rejects_soft_limit_above_hard_limit() {
            let rejection = check_delta_admissible(
                &KnobDelta::UpdatePressureThreshold {
                    update: ResourcePressureUpdate::WramHot {
                        limit: PressureLimit {
                            soft: 4097,
                            hard: 4096,
                        },
                    },
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("soft above hard rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::BeyondBounds {
                    knob: CompileKnobId::ScheduleResourcePressure,
                    ..
                }
            ));
        }

        #[test]
        fn pressure_update_rejects_non_monotone_threshold_change() {
            let rejection = check_delta_admissible(
                &KnobDelta::UpdatePressureThreshold {
                    update: ResourcePressureUpdate::WramHot {
                        limit: PressureLimit {
                            soft: 1024,
                            hard: 2048,
                        },
                    },
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect_err("lower pressure threshold rejects");

            assert!(matches!(
                rejection,
                DeltaRejection::NotMonotone {
                    knob: CompileKnobId::ScheduleResourcePressure,
                    ..
                }
            ));
        }

        #[test]
        fn pressure_update_accepts_mutable_thresholds() {
            check_delta_admissible(
                &KnobDelta::UpdatePressureThreshold {
                    update: ResourcePressureUpdate::WramHot {
                        limit: PressureLimit {
                            soft: 7000,
                            hard: 8192,
                        },
                    },
                },
                &compile_knobs_fixture(),
                &RepairPolicy::for_profile(RepairPolicyProfile::Default),
                ObservabilityMode::Flexible,
            )
            .expect("valid pressure update is repair-mutable");
        }

        #[test]
        fn pressure_thresholds_fall_back_to_profile_defaults_without_runtime_budget() {
            let profile =
                load_compile_profile_spec(BRINGUP_COMPILE_PROFILE_TOML).expect("profile loads");
            let objective = CompileObjective {
                service: None,
                max_cycles_per_token: None,
                max_bank_switches_per_token: None,
                max_sram_page_switches_per_token: None,
                ..objective_fixture()
            };
            let resolved = resolve_resource_pressure_thresholds(
                &profile,
                None,
                &objective,
                &profile.trace_budget,
            )
            .expect("thresholds resolve without runtime chrome budget");

            assert_limit(resolved.thresholds.wram_hot, 4096);
            assert_limit(resolved.thresholds.hram_hot, 96);
            assert_limit(resolved.thresholds.bank0_rom, 4096);
            assert_limit(resolved.thresholds.switchable_rom_window, 16 * 1024);
            assert_limit(resolved.thresholds.sram_window, 8 * 1024);
            assert_limit(resolved.thresholds.slice_cycles, 8192);
            assert_limit(resolved.thresholds.interrupt_latency, 2048);
            assert_limit(
                resolved.thresholds.trace_bytes_per_frame,
                profile.trace_budget.max_bytes_per_frame,
            );
            assert_limit(resolved.thresholds.persist_bytes_per_frame, 512);
            assert_limit(resolved.thresholds.overlay_installs_per_frame, 1);
            assert_limit(resolved.thresholds.bank_switches_per_token, 8);
            assert_limit(resolved.thresholds.sram_page_switches_per_token, 4);
            assert!(resolved.provenance.iter().all(|entry| {
                entry.path.knob == CompileKnobId::ScheduleResourcePressure
                    && !entry.chain.is_empty()
            }));
        }

        fn assert_limit<T>(limit: PressureLimit<T>, expected_hard: T)
        where
            T: Copy + PartialOrd + std::fmt::Debug + PartialEq,
        {
            assert_eq!(limit.hard, expected_hard);
            assert!(limit.soft <= limit.hard);
        }
    }

    #[derive(Clone, Default)]
    struct TraceCapture {
        events: Arc<Mutex<Vec<CapturedEvent>>>,
    }

    #[derive(Debug)]
    struct CapturedEvent {
        fields: BTreeMap<String, String>,
    }

    impl CapturedEvent {
        fn field(&self, name: &str) -> Option<&str> {
            self.fields.get(name).map(String::as_str)
        }
    }

    impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
    where
        S: tracing::Subscriber,
    {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let mut visitor = TraceFieldVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("capture lock is healthy")
                .push(CapturedEvent {
                    fields: visitor.fields,
                });
        }
    }

    #[derive(Default)]
    struct TraceFieldVisitor {
        fields: BTreeMap<String, String>,
    }

    fn installed_trace_capture() -> TraceCapture {
        static TRACE_CAPTURE: OnceLock<TraceCapture> = OnceLock::new();
        static TRACE_CAPTURE_INIT: OnceLock<()> = OnceLock::new();

        let capture = TRACE_CAPTURE.get_or_init(TraceCapture::default).clone();
        TRACE_CAPTURE_INIT.get_or_init(|| {
            let subscriber = tracing_subscriber::registry()
                .with(LevelFilter::INFO)
                .with(capture.clone());
            tracing::subscriber::set_global_default(subscriber)
                .expect("profile telemetry capture subscriber installs once");
        });
        capture
            .events
            .lock()
            .expect("capture lock is healthy")
            .clear();
        tracing::callsite::rebuild_interest_cache();
        capture
    }

    impl tracing::field::Visit for TraceFieldVisitor {
        fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn fmt::Debug) {
            self.fields
                .insert(field.name().to_owned(), format!("{value:?}"));
        }

        fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
            self.fields
                .insert(field.name().to_owned(), value.to_owned());
        }

        fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
            self.fields
                .insert(field.name().to_owned(), value.to_string());
        }
    }
}
