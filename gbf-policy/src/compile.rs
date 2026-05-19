//! Compile-request and resolved-policy schema.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_foundation::{
    BlobRef, CompileProfileId, Hash256, TargetProfileId, canonical_json::DomainHash,
};
use gbf_hw::calibration::CalibrationSetRef;
use gbf_hw::target::TargetProfile;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::budget::RuntimeChromeBudget;
use crate::canonical::domain_hash;
use crate::objective::{CompileObjective, RiskPolicy};
use crate::repair::{RepairPolicy, RepairProposalId};

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

impl ResolvedCompilePolicy {
    #[must_use]
    pub fn canonical_hash(&self) -> Hash256 {
        let domain = DomainHash::new(
            "gbf-policy",
            "ResolvedCompilePolicy",
            "policy_resolution.v1",
            "1",
        );
        domain
            .hash(self)
            .expect("ResolvedCompilePolicy canonical hash is computable")
    }
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
    pub values: CompileKnobPartialValues,
    pub bounds: CompileKnobPartialBounds,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum ConstraintOperation {
    SeedDefault,
    TightenBound,
    ApplyPreference,
    ApplyHardConstraint,
    ApplyOverride,
    ApplyCalibration,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnob {
    pub observability: ObservabilityMode,
    pub probe_level: ProbeCollectionLevel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationKnobBounds {
    pub max_probe_level: ProbeCollectionLevel,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnob {
    pub page_aggression: SramPageAggression,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SramKnobBounds {
    pub max_page_aggression: SramPageAggression,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnob {
    pub tile_search: ScheduleTileSearch,
    pub slice_coarsening: ScheduleSliceCoarsening,
    pub resource_pressure: ScheduleResourcePressure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleKnobBounds {
    pub max_tile_search: ScheduleTileSearch,
    pub max_slice_coarsening: ScheduleSliceCoarsening,
    pub max_resource_pressure: ScheduleResourcePressure,
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
    fn is_monotone_successor_of(&self, previous: &Self) -> bool;
}

macro_rules! monotone_enum {
    ($ty:ty) => {
        impl MonotoneKnob for $ty {
            fn is_monotone_successor_of(&self, previous: &Self) -> bool {
                self >= previous
            }
        }
    };
}

monotone_enum!(PlacementProfile);
monotone_enum!(ProbeCollectionLevel);
monotone_enum!(ReductionPlanCeiling);
monotone_enum!(StorageMaterialization);
monotone_enum!(SramPageAggression);
monotone_enum!(RomKernelResidencyBias);
monotone_enum!(RomKernelDuplicationBias);
monotone_enum!(OverlayPromotion);
monotone_enum!(ScheduleTileSearch);
monotone_enum!(ScheduleSliceCoarsening);
monotone_enum!(ScheduleResourcePressure);

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
        self.probe_level
            .is_monotone_successor_of(&previous.probe_level)
    }
}

impl MonotoneKnob for ObservationKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_probe_level <= previous.max_probe_level
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
    }
}

impl MonotoneKnob for SramKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_page_aggression <= previous.max_page_aggression
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
    }
}

impl MonotoneKnob for ScheduleKnobBounds {
    fn is_monotone_successor_of(&self, previous: &Self) -> bool {
        self.max_tile_search <= previous.max_tile_search
            && self.max_slice_coarsening <= previous.max_slice_coarsening
            && self.max_resource_pressure <= previous.max_resource_pressure
    }
}

#[must_use]
pub const fn canonical_default_bounds_fixture() -> CompileKnobBounds {
    CompileKnobBounds {
        placement: PlacementKnobBounds {
            max_profile: PlacementProfile::PackedExperts,
        },
        observation: ObservationKnobBounds {
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
        serde_json::json!({
            "placement": {"profile": {"kind": "Budgeted"}},
            "observation": {
                "observability": {"kind": "Invariant"},
                "probe_level": {"kind": "Operational"}
            },
            "range": {"reduction_ceiling": {"kind": "Conservative"}},
            "storage": {"materialization": {"kind": "RecomputePureValues"}},
            "sram": {"page_aggression": {"kind": "PackCold"}},
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
        })
    }

    fn default_bounds_json() -> serde_json::Value {
        serde_json::json!({
            "placement": {"max_profile": {"kind": "PackedExperts"}},
            "observation": {"max_probe_level": {"kind": "Verbose"}},
            "range": {"max_reduction_ceiling": {"kind": "Adaptive"}},
            "storage": {"max_materialization": {"kind": "SpillColdValues"}},
            "sram": {"max_page_aggression": {"kind": "MinimizeResident"}},
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
        })
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
                max_probe_level: ProbeCollectionLevel::Verbose,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Operational,
            })
        );
        assert!(
            ObservationKnobBounds {
                max_probe_level: ProbeCollectionLevel::Operational,
            }
            .is_monotone_successor_of(&ObservationKnobBounds {
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
            }
            .is_monotone_successor_of(&SramKnobBounds {
                max_page_aggression: SramPageAggression::PackCold,
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
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Coarse,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
            })
        );
        assert!(
            !ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::FitFirst,
            }
            .is_monotone_successor_of(&ScheduleKnobBounds {
                max_tile_search: ScheduleTileSearch::Local,
                max_slice_coarsening: ScheduleSliceCoarsening::Balanced,
                max_resource_pressure: ScheduleResourcePressure::Balanced,
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
