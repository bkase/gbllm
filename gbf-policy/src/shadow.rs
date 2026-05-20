//! F-S5 shadow compile schema and path helpers.

use std::fmt;
use std::path::PathBuf;

use gbf_foundation::Hash256;
use serde::de::{Error as DeError, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::canonical_json_bytes;

pub const S5_SHADOW_COMPILE_SAMPLE_SCHEMA: &str = "s5_shadow_compile_sample.v1";
pub const H13_SHADOW_FINAL_STRICT_PASS_MAX_BYTES: u64 = 1024;
pub const H13_SHADOW_FINAL_WARNING_MAX_BYTES: u64 = 2048;

pub const S5_SHADOW_PIPELINE_STAGES: [&str; 16] = [
    "QuantGraph",
    "StaticBudgetReport",
    "GbInferIR",
    "ObservationPlan",
    "RangePlan",
    "StoragePlan",
    "SramPagePlan",
    "RomWindowPlan",
    "OverlayPlan",
    "ArenaPlan",
    "GbSchedIR",
    "ResourceStateValidation",
    "AsmIR",
    "ReachabilityValidation",
    "PlacedRom",
    "EncodedRom",
];

pub const S5_SHADOW_CADENCE_STEPS: [ShadowStep; 5] = [
    ShadowStep::new(4000),
    ShadowStep::new(8000),
    ShadowStep::new(12000),
    ShadowStep::new(16000),
    ShadowStep::new(20000),
];

/// Training step at which a real shadow compile cadence sample was emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ShadowStep(u32);

impl ShadowStep {
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for ShadowStep {
    fn from(value: u32) -> Self {
        Self::new(value)
    }
}

impl From<ShadowStep> for u32 {
    fn from(value: ShadowStep) -> Self {
        value.get()
    }
}

impl fmt::Display for ShadowStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Discriminates cadence samples from the Phase E final shadow emission.
///
/// JSON is intentionally field-discriminated:
/// `{"step": 20000}` for cadence and `{"phase_e_final": true}` for the
/// final Phase E hand-off. This prevents the final emission from resolving to
/// the cadence `step-20000.json` path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ShadowEmissionId {
    Step(ShadowStep),
    PhaseEFinal,
}

impl ShadowEmissionId {
    #[must_use]
    pub const fn cadence(step: ShadowStep) -> Self {
        Self::Step(step)
    }

    #[must_use]
    pub const fn phase_e_final() -> Self {
        Self::PhaseEFinal
    }

    #[must_use]
    pub const fn step(self) -> Option<ShadowStep> {
        match self {
            Self::Step(step) => Some(step),
            Self::PhaseEFinal => None,
        }
    }

    #[must_use]
    pub fn file_name(self) -> String {
        match self {
            Self::Step(step) => format!("step-{}.json", step.get()),
            Self::PhaseEFinal => "phase-e-final.json".to_owned(),
        }
    }
}

impl Serialize for ShadowEmissionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Step(step) => map.serialize_entry("step", step)?,
            Self::PhaseEFinal => map.serialize_entry("phase_e_final", &true)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ShadowEmissionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ShadowEmissionIdVisitor;

        impl<'de> Visitor<'de> for ShadowEmissionIdVisitor {
            type Value = ShadowEmissionId;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(r#"{"step": N} or {"phase_e_final": true}"#)
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let Some(key) = map.next_key::<String>()? else {
                    return Err(A::Error::custom("ShadowEmissionId cannot be empty"));
                };

                let emission = match key.as_str() {
                    "step" => Self::Value::Step(map.next_value()?),
                    "phase_e_final" => {
                        let value = map.next_value::<bool>()?;
                        if !value {
                            return Err(A::Error::custom("phase_e_final must be true"));
                        }
                        Self::Value::PhaseEFinal
                    }
                    other => {
                        return Err(A::Error::unknown_field(other, &["step", "phase_e_final"]));
                    }
                };

                if map.next_key::<String>()?.is_some() {
                    return Err(A::Error::custom(
                        "ShadowEmissionId must contain exactly one discriminator field",
                    ));
                }

                Ok(emission)
            }
        }

        deserializer.deserialize_map(ShadowEmissionIdVisitor)
    }
}

/// Real Fit-axis shadow compile sample produced by the full pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowCompileSampleReal {
    pub schema: String,
    pub variant: String,
    pub seed: u64,
    pub emission_id: ShadowEmissionId,
    pub stages_executed: Vec<String>,
    pub shadow_byte_cost: u32,
    pub shadow_kernel_count: u32,
    pub shadow_compile_ok: bool,
    pub shadow_compile_skipped: Option<String>,
    pub failure_stage: Option<String>,
    pub fits_envelope: bool,
    pub reachability_cert_valid: bool,
    pub resource_state_cert_valid: bool,
    pub shadow_latency_proxy_cycles: u64,
    pub shadow_energy_proxy_units: u64,
    pub compiler_feedback_sha: Hash256,
    pub sample_self_hash: Hash256,
}

impl ShadowCompileSampleReal {
    #[must_use]
    pub fn shadow_file_name(&self) -> String {
        self.emission_id.file_name()
    }

    #[must_use]
    pub fn shadow_path(&self) -> PathBuf {
        shadow_compile_sample_real_path(self.seed, self.emission_id)
    }

    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        canonical_json_bytes(self)
    }
}

/// Which SHR-1 branch a caller expects this sample to satisfy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShadowCompileSampleExpectation {
    Healthy,
    BrokenNegativeControl,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shr1ValidationError {
    HealthyCompileNotOk,
    HealthyStagesMismatch {
        expected: Vec<&'static str>,
        observed: Vec<String>,
    },
    HealthyFailureStagePresent {
        failure_stage: String,
    },
    BrokenCompileOk,
    BrokenDiagnosticMissing,
    BrokenFailureStageMissing,
    BrokenNoAttemptedStages,
    BrokenAttemptedStagesNotPrefix {
        observed: Vec<String>,
    },
    BrokenFailureStageUnknown {
        failure_stage: String,
    },
}

impl fmt::Display for Shr1ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HealthyCompileNotOk => write!(f, "healthy SHR-1 sample must have ok=true"),
            Self::HealthyStagesMismatch { .. } => {
                write!(f, "healthy SHR-1 sample stages do not match S5 pipeline")
            }
            Self::HealthyFailureStagePresent { failure_stage } => {
                write!(
                    f,
                    "healthy SHR-1 sample unexpectedly carried failure_stage {failure_stage}"
                )
            }
            Self::BrokenCompileOk => write!(f, "broken SHR-1 sample must have ok=false"),
            Self::BrokenDiagnosticMissing => {
                write!(f, "broken SHR-1 sample must carry a diagnostic")
            }
            Self::BrokenFailureStageMissing => {
                write!(f, "broken SHR-1 sample must carry failure_stage")
            }
            Self::BrokenNoAttemptedStages => {
                write!(f, "broken SHR-1 sample must record attempted stages")
            }
            Self::BrokenAttemptedStagesNotPrefix { .. } => {
                write!(
                    f,
                    "broken SHR-1 sample attempted stages must be an S5 pipeline prefix"
                )
            }
            Self::BrokenFailureStageUnknown { failure_stage } => {
                write!(
                    f,
                    "broken SHR-1 sample failure_stage {failure_stage} is not in S5 pipeline"
                )
            }
        }
    }
}

impl std::error::Error for Shr1ValidationError {}

pub fn validate_shr1_shadow_sample(
    sample: &ShadowCompileSampleReal,
    expectation: ShadowCompileSampleExpectation,
) -> Result<(), Shr1ValidationError> {
    match expectation {
        ShadowCompileSampleExpectation::Healthy => validate_shr1_healthy(sample),
        ShadowCompileSampleExpectation::BrokenNegativeControl => validate_shr1_broken(sample),
    }
}

fn validate_shr1_healthy(sample: &ShadowCompileSampleReal) -> Result<(), Shr1ValidationError> {
    if !sample.shadow_compile_ok {
        return Err(Shr1ValidationError::HealthyCompileNotOk);
    }
    if !stages_equal_pinned_pipeline(&sample.stages_executed) {
        return Err(Shr1ValidationError::HealthyStagesMismatch {
            expected: S5_SHADOW_PIPELINE_STAGES.to_vec(),
            observed: sample.stages_executed.clone(),
        });
    }
    if let Some(failure_stage) = non_empty_optional(&sample.failure_stage) {
        return Err(Shr1ValidationError::HealthyFailureStagePresent {
            failure_stage: failure_stage.to_owned(),
        });
    }
    Ok(())
}

fn validate_shr1_broken(sample: &ShadowCompileSampleReal) -> Result<(), Shr1ValidationError> {
    if sample.shadow_compile_ok {
        return Err(Shr1ValidationError::BrokenCompileOk);
    }
    if non_empty_optional(&sample.shadow_compile_skipped).is_none() {
        return Err(Shr1ValidationError::BrokenDiagnosticMissing);
    }
    let Some(failure_stage) = non_empty_optional(&sample.failure_stage) else {
        return Err(Shr1ValidationError::BrokenFailureStageMissing);
    };
    if sample.stages_executed.is_empty() {
        return Err(Shr1ValidationError::BrokenNoAttemptedStages);
    }
    if !stages_are_pinned_pipeline_prefix(&sample.stages_executed) {
        return Err(Shr1ValidationError::BrokenAttemptedStagesNotPrefix {
            observed: sample.stages_executed.clone(),
        });
    }
    if !S5_SHADOW_PIPELINE_STAGES.contains(&failure_stage) {
        return Err(Shr1ValidationError::BrokenFailureStageUnknown {
            failure_stage: failure_stage.to_owned(),
        });
    }
    Ok(())
}

fn stages_equal_pinned_pipeline(stages: &[String]) -> bool {
    stages.len() == S5_SHADOW_PIPELINE_STAGES.len()
        && stages
            .iter()
            .map(String::as_str)
            .eq(S5_SHADOW_PIPELINE_STAGES)
}

fn stages_are_pinned_pipeline_prefix(stages: &[String]) -> bool {
    stages.len() <= S5_SHADOW_PIPELINE_STAGES.len()
        && stages
            .iter()
            .map(String::as_str)
            .eq(S5_SHADOW_PIPELINE_STAGES.iter().copied().take(stages.len()))
}

fn non_empty_optional(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.trim().is_empty())
}

/// H13 byte-cost gap classification for the step-20000 shadow sample versus
/// the Phase E final EncodedRom byte cost.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum H13ShadowFinalByteCostStatus {
    /// `|shadow_at_20000 - final.encoded_rom_byte_cost| <= 1024`.
    StrictPass,
    /// `1024 < |shadow_at_20000 - final.encoded_rom_byte_cost| <= 2048`.
    WarningBand,
    /// `|shadow_at_20000 - final.encoded_rom_byte_cost| > 2048`.
    Refuted,
}

/// H13 byte-cost gap result carrying the absolute byte delta for audit logs
/// and future S5 outcome dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct H13ShadowFinalByteCostGap {
    pub delta_bytes: u64,
    pub status: H13ShadowFinalByteCostStatus,
}

impl H13ShadowFinalByteCostGap {
    #[must_use]
    pub const fn is_strict_pass(self) -> bool {
        matches!(self.status, H13ShadowFinalByteCostStatus::StrictPass)
    }

    #[must_use]
    pub const fn is_warning_band(self) -> bool {
        matches!(self.status, H13ShadowFinalByteCostStatus::WarningBand)
    }

    #[must_use]
    pub const fn is_refuted(self) -> bool {
        matches!(self.status, H13ShadowFinalByteCostStatus::Refuted)
    }
}

#[must_use]
pub const fn h13_shadow_final_byte_cost_status(delta_bytes: u64) -> H13ShadowFinalByteCostStatus {
    if delta_bytes <= H13_SHADOW_FINAL_STRICT_PASS_MAX_BYTES {
        H13ShadowFinalByteCostStatus::StrictPass
    } else if delta_bytes <= H13_SHADOW_FINAL_WARNING_MAX_BYTES {
        H13ShadowFinalByteCostStatus::WarningBand
    } else {
        H13ShadowFinalByteCostStatus::Refuted
    }
}

#[must_use]
pub fn h13_shadow_final_byte_cost_gap(
    shadow_at_20000_byte_cost: u64,
    final_encoded_rom_byte_cost: u64,
) -> H13ShadowFinalByteCostGap {
    let delta_bytes = shadow_at_20000_byte_cost.abs_diff(final_encoded_rom_byte_cost);
    H13ShadowFinalByteCostGap {
        delta_bytes,
        status: h13_shadow_final_byte_cost_status(delta_bytes),
    }
}

#[must_use]
pub fn h13_shadow_sample_final_byte_cost_gap(
    shadow_at_20000: &ShadowCompileSampleReal,
    final_encoded_rom_byte_cost: u64,
) -> H13ShadowFinalByteCostGap {
    h13_shadow_final_byte_cost_gap(
        u64::from(shadow_at_20000.shadow_byte_cost),
        final_encoded_rom_byte_cost,
    )
}

#[must_use]
pub fn shadow_compile_sample_real_path(seed: u64, emission_id: ShadowEmissionId) -> PathBuf {
    PathBuf::from("experiments")
        .join("S5")
        .join("runs")
        .join(format!("seed-{seed}"))
        .join("shadow")
        .join(emission_id.file_name())
}

#[must_use]
pub fn shadow_compile_sample_real_emission_order() -> [ShadowEmissionId; 6] {
    [
        ShadowEmissionId::Step(S5_SHADOW_CADENCE_STEPS[0]),
        ShadowEmissionId::Step(S5_SHADOW_CADENCE_STEPS[1]),
        ShadowEmissionId::Step(S5_SHADOW_CADENCE_STEPS[2]),
        ShadowEmissionId::Step(S5_SHADOW_CADENCE_STEPS[3]),
        ShadowEmissionId::Step(S5_SHADOW_CADENCE_STEPS[4]),
        ShadowEmissionId::PhaseEFinal,
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use serde_json::json;

    use super::*;

    #[test]
    fn shadow_emission_id_round_trip() {
        let cadence = ShadowEmissionId::cadence(ShadowStep::new(20000));
        let phase_e_final = ShadowEmissionId::phase_e_final();

        assert_eq!(
            serde_json::to_value(cadence).unwrap(),
            json!({"step": 20000})
        );
        assert_eq!(
            serde_json::from_value::<ShadowEmissionId>(json!({"step": 20000})).unwrap(),
            cadence
        );

        assert_eq!(
            serde_json::to_value(phase_e_final).unwrap(),
            json!({"phase_e_final": true})
        );
        assert_eq!(
            serde_json::from_value::<ShadowEmissionId>(json!({"phase_e_final": true})).unwrap(),
            phase_e_final
        );

        assert!(
            serde_json::from_value::<ShadowEmissionId>(json!({"phase_e_final": false})).is_err()
        );
        assert!(
            serde_json::from_value::<ShadowEmissionId>(
                json!({"step": 20000, "phase_e_final": true})
            )
            .is_err()
        );
    }

    #[test]
    fn shadow_compile_sample_real_phase_e_final_path() {
        assert_eq!(
            shadow_compile_sample_real_path(0, ShadowEmissionId::phase_e_final()),
            Path::new("experiments/S5/runs/seed-0/shadow/phase-e-final.json")
        );
    }

    #[test]
    fn shadow_compile_sample_real_cadence_path() {
        assert_eq!(
            shadow_compile_sample_real_path(0, ShadowEmissionId::cadence(ShadowStep::new(20000))),
            Path::new("experiments/S5/runs/seed-0/shadow/step-20000.json")
        );
    }

    #[test]
    fn six_emissions_per_seed_distinct_paths() {
        let paths: BTreeSet<_> = shadow_compile_sample_real_emission_order()
            .into_iter()
            .map(|emission_id| shadow_compile_sample_real_path(7, emission_id))
            .collect();

        assert_eq!(paths.len(), 6);
        assert!(paths.contains(Path::new(
            "experiments/S5/runs/seed-7/shadow/step-20000.json"
        )));
        assert!(paths.contains(Path::new(
            "experiments/S5/runs/seed-7/shadow/phase-e-final.json"
        )));
    }

    #[test]
    fn shadow_compile_sample_real_emission_order_is_cadence_then_phase_e_final() {
        assert_eq!(
            shadow_compile_sample_real_emission_order(),
            [
                ShadowEmissionId::cadence(ShadowStep::new(4000)),
                ShadowEmissionId::cadence(ShadowStep::new(8000)),
                ShadowEmissionId::cadence(ShadowStep::new(12000)),
                ShadowEmissionId::cadence(ShadowStep::new(16000)),
                ShadowEmissionId::cadence(ShadowStep::new(20000)),
                ShadowEmissionId::phase_e_final(),
            ]
        );
    }

    #[test]
    fn shadow_compile_sample_real_uses_emission_id_not_step() {
        let mut value = serde_json::to_value(sample(ShadowEmissionId::phase_e_final())).unwrap();
        assert!(value.get("emission_id").is_some());
        assert!(value.get("step").is_none());

        value
            .as_object_mut()
            .unwrap()
            .insert("step".to_owned(), json!(20000));
        assert!(serde_json::from_value::<ShadowCompileSampleReal>(value).is_err());
    }

    #[test]
    fn shadow_compile_sample_real_canonical_json_sorts_emission_id() {
        let sample = sample(ShadowEmissionId::cadence(ShadowStep::new(4000)));
        let canonical = String::from_utf8(sample.canonical_json_bytes().unwrap()).unwrap();
        let value: serde_json::Value = serde_json::from_str(&canonical).unwrap();

        assert!(canonical.contains(r#""emission_id":{"step":4000}"#));
        assert!(value.get("emission_id").is_some());
        assert!(value.get("step").is_none());
    }

    #[test]
    fn h13_shadow_final_gap_delta_1024_is_strict_pass() {
        assert_eq!(
            h13_shadow_final_byte_cost_gap(10_000, 11_024),
            H13ShadowFinalByteCostGap {
                delta_bytes: 1024,
                status: H13ShadowFinalByteCostStatus::StrictPass,
            }
        );
    }

    #[test]
    fn h13_shadow_final_gap_delta_1025_is_warning() {
        assert_eq!(
            h13_shadow_final_byte_cost_gap(10_000, 11_025),
            H13ShadowFinalByteCostGap {
                delta_bytes: 1025,
                status: H13ShadowFinalByteCostStatus::WarningBand,
            }
        );
    }

    #[test]
    fn h13_shadow_final_gap_delta_2048_is_warning() {
        assert_eq!(
            h13_shadow_final_byte_cost_gap(10_000, 12_048),
            H13ShadowFinalByteCostGap {
                delta_bytes: 2048,
                status: H13ShadowFinalByteCostStatus::WarningBand,
            }
        );
    }

    #[test]
    fn h13_shadow_final_gap_delta_2049_is_refuted() {
        assert_eq!(
            h13_shadow_final_byte_cost_gap(10_000, 12_049),
            H13ShadowFinalByteCostGap {
                delta_bytes: 2049,
                status: H13ShadowFinalByteCostStatus::Refuted,
            }
        );
    }

    #[test]
    fn h13_shadow_final_gap_is_absolute() {
        assert_eq!(
            h13_shadow_final_byte_cost_gap(12_049, 10_000).status,
            H13ShadowFinalByteCostStatus::Refuted
        );
    }

    #[test]
    fn h13_shadow_sample_final_gap_uses_shadow_byte_cost() {
        let sample = ShadowCompileSampleReal {
            shadow_byte_cost: 10_000,
            ..sample(ShadowEmissionId::cadence(ShadowStep::new(20000)))
        };

        assert_eq!(
            h13_shadow_sample_final_byte_cost_gap(&sample, 11_025),
            H13ShadowFinalByteCostGap {
                delta_bytes: 1025,
                status: H13ShadowFinalByteCostStatus::WarningBand,
            }
        );
    }

    #[test]
    fn shr1_healthy_branch_passes() {
        let sample = ShadowCompileSampleReal {
            stages_executed: pinned_stage_strings(),
            shadow_compile_ok: true,
            shadow_compile_skipped: None,
            failure_stage: None,
            ..sample(ShadowEmissionId::cadence(ShadowStep::new(4000)))
        };

        assert_eq!(
            validate_shr1_shadow_sample(&sample, ShadowCompileSampleExpectation::Healthy),
            Ok(())
        );
    }

    #[test]
    fn shr1_broken_branch_passes() {
        let sample = broken_sample();

        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Ok(())
        );
    }

    #[test]
    fn shr1_broken_missing_diagnostic_or_failure_stage_fails() {
        let missing_diagnostic = ShadowCompileSampleReal {
            shadow_compile_skipped: None,
            ..broken_sample()
        };
        assert_eq!(
            validate_shr1_shadow_sample(
                &missing_diagnostic,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenDiagnosticMissing)
        );

        let missing_failure_stage = ShadowCompileSampleReal {
            failure_stage: None,
            ..broken_sample()
        };
        assert_eq!(
            validate_shr1_shadow_sample(
                &missing_failure_stage,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenFailureStageMissing)
        );
    }

    #[test]
    fn shr1_healthy_stage_mismatch_fails() {
        let observed = vec!["ImportQuantGraph".to_owned()];
        let sample = ShadowCompileSampleReal {
            stages_executed: observed.clone(),
            shadow_compile_ok: true,
            shadow_compile_skipped: None,
            failure_stage: None,
            ..sample(ShadowEmissionId::cadence(ShadowStep::new(4000)))
        };

        assert_eq!(
            validate_shr1_shadow_sample(&sample, ShadowCompileSampleExpectation::Healthy),
            Err(Shr1ValidationError::HealthyStagesMismatch {
                expected: S5_SHADOW_PIPELINE_STAGES.to_vec(),
                observed,
            })
        );
    }

    #[test]
    fn shr1_healthy_failure_stage_present_fails() {
        let sample = ShadowCompileSampleReal {
            stages_executed: pinned_stage_strings(),
            shadow_compile_ok: true,
            shadow_compile_skipped: None,
            failure_stage: Some("RomWindowPlan".to_owned()),
            ..sample(ShadowEmissionId::cadence(ShadowStep::new(4000)))
        };

        assert_eq!(
            validate_shr1_shadow_sample(&sample, ShadowCompileSampleExpectation::Healthy),
            Err(Shr1ValidationError::HealthyFailureStagePresent {
                failure_stage: "RomWindowPlan".to_owned(),
            })
        );
    }

    #[test]
    fn shr1_broken_compile_ok_fails() {
        let sample = ShadowCompileSampleReal {
            shadow_compile_ok: true,
            ..broken_sample()
        };

        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenCompileOk)
        );
    }

    #[test]
    fn shr1_broken_no_attempted_stages_fails() {
        let sample = ShadowCompileSampleReal {
            stages_executed: Vec::new(),
            ..broken_sample()
        };

        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenNoAttemptedStages)
        );
    }

    #[test]
    fn shr1_broken_attempted_stages_not_prefix_fails() {
        let observed = vec!["StaticBudgetReport".to_owned()];
        let sample = ShadowCompileSampleReal {
            stages_executed: observed.clone(),
            ..broken_sample()
        };

        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenAttemptedStagesNotPrefix { observed })
        );
    }

    #[test]
    fn shr1_broken_failure_stage_unknown_fails() {
        let sample = ShadowCompileSampleReal {
            failure_stage: Some("FutureShadowStage".to_owned()),
            ..broken_sample()
        };

        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Err(Shr1ValidationError::BrokenFailureStageUnknown {
                failure_stage: "FutureShadowStage".to_owned(),
            })
        );
    }

    #[test]
    fn shr1_broken_negative_control_fixture_is_valid() {
        let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../fixtures/s5/shadow/broken_negative_control.sample.json");
        let fixture = std::fs::read_to_string(fixture_path).unwrap();
        let sample: ShadowCompileSampleReal = serde_json::from_str(&fixture).unwrap();

        assert_eq!(sample.failure_stage.as_deref(), Some("RomWindowPlan"));
        assert_eq!(
            validate_shr1_shadow_sample(
                &sample,
                ShadowCompileSampleExpectation::BrokenNegativeControl
            ),
            Ok(())
        );
    }

    fn sample(emission_id: ShadowEmissionId) -> ShadowCompileSampleReal {
        ShadowCompileSampleReal {
            schema: S5_SHADOW_COMPILE_SAMPLE_SCHEMA.to_owned(),
            variant: "boundedkv".to_owned(),
            seed: 0,
            emission_id,
            stages_executed: vec![
                "ImportQuantGraph".to_owned(),
                "StaticBudgetReport".to_owned(),
            ],
            shadow_byte_cost: 1234,
            shadow_kernel_count: 9,
            shadow_compile_ok: true,
            shadow_compile_skipped: None,
            failure_stage: None,
            fits_envelope: true,
            reachability_cert_valid: true,
            resource_state_cert_valid: true,
            shadow_latency_proxy_cycles: 55,
            shadow_energy_proxy_units: 89,
            compiler_feedback_sha: Hash256::ZERO,
            sample_self_hash: Hash256::ZERO,
        }
    }

    fn broken_sample() -> ShadowCompileSampleReal {
        ShadowCompileSampleReal {
            stages_executed: S5_SHADOW_PIPELINE_STAGES[..7]
                .iter()
                .map(ToString::to_string)
                .collect(),
            shadow_byte_cost: u32::MAX,
            shadow_kernel_count: 0,
            shadow_compile_ok: false,
            shadow_compile_skipped: Some(
                "broken-negative-control: out-of-WRAM placement fixture".to_owned(),
            ),
            failure_stage: Some("RomWindowPlan".to_owned()),
            fits_envelope: false,
            reachability_cert_valid: false,
            resource_state_cert_valid: false,
            shadow_latency_proxy_cycles: 0,
            shadow_energy_proxy_units: 0,
            ..sample(ShadowEmissionId::cadence(ShadowStep::new(4000)))
        }
    }

    fn pinned_stage_strings() -> Vec<String> {
        S5_SHADOW_PIPELINE_STAGES
            .iter()
            .map(ToString::to_string)
            .collect()
    }
}
