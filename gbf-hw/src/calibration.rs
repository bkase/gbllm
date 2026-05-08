//! Layered calibration schema shared by policy, compiler, bench, and reports.

use std::fmt;

use gbf_foundation::{
    CalibrationCohortId, Hash256, KernelCalibrationId, KernelSpecId, PlatformCalibrationId,
    RuntimeCalibrationId, SemVer, TargetFamilyId, TargetProfileId,
};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub const STRONG_CONFIDENCE_MIN_SAMPLES: u32 = 1_000;
pub const REASONABLE_CONFIDENCE_MIN_SAMPLES: u32 = 100;

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CycleDistributionRepr")]
pub struct CycleDistribution {
    mean: f32,
    p50: u32,
    p90: u32,
    p99: u32,
}

impl CycleDistribution {
    pub fn new(mean: f32, p50: u32, p90: u32, p99: u32) -> Result<Self, CalibrationError> {
        if !mean.is_finite() || mean < 0.0 {
            return Err(CalibrationError::InvalidCycleDistribution);
        }
        if !(p50 <= p90 && p90 <= p99) {
            return Err(CalibrationError::InvalidCycleDistribution);
        }

        Ok(Self {
            mean,
            p50,
            p90,
            p99,
        })
    }

    #[must_use]
    pub const fn mean(&self) -> f32 {
        self.mean
    }

    #[must_use]
    pub const fn p50(&self) -> u32 {
        self.p50
    }

    #[must_use]
    pub const fn p90(&self) -> u32 {
        self.p90
    }

    #[must_use]
    pub const fn p99(&self) -> u32 {
        self.p99
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
struct CycleDistributionRepr {
    mean: f32,
    p50: u32,
    p90: u32,
    p99: u32,
}

impl TryFrom<CycleDistributionRepr> for CycleDistribution {
    type Error = CalibrationError;

    fn try_from(value: CycleDistributionRepr) -> Result<Self, Self::Error> {
        Self::new(value.mean, value.p50, value.p90, value.p99)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
#[repr(u8)]
pub enum CalibrationConfidenceClass {
    None = 0,
    Weak = 1,
    Reasonable = 2,
    Strong = 3,
}

impl CalibrationConfidenceClass {
    #[must_use]
    pub const fn rank(self) -> u8 {
        self as u8
    }
}

impl Serialize for CalibrationConfidenceClass {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        #[serde(rename_all = "PascalCase")]
        enum CalibrationConfidenceClassTag {
            None,
            Weak,
            Reasonable,
            Strong,
        }

        #[derive(Serialize)]
        struct TaggedCalibrationConfidenceClass {
            kind: CalibrationConfidenceClassTag,
        }

        let kind = match self {
            Self::None => CalibrationConfidenceClassTag::None,
            Self::Weak => CalibrationConfidenceClassTag::Weak,
            Self::Reasonable => CalibrationConfidenceClassTag::Reasonable,
            Self::Strong => CalibrationConfidenceClassTag::Strong,
        };
        TaggedCalibrationConfidenceClass { kind }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for CalibrationConfidenceClass {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum CalibrationConfidenceClassTag {
            None,
            Weak,
            Reasonable,
            Strong,
        }

        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct TaggedCalibrationConfidenceClass {
            kind: CalibrationConfidenceClassTag,
        }

        Ok(
            match TaggedCalibrationConfidenceClass::deserialize(deserializer)?.kind {
                CalibrationConfidenceClassTag::None => Self::None,
                CalibrationConfidenceClassTag::Weak => Self::Weak,
                CalibrationConfidenceClassTag::Reasonable => Self::Reasonable,
                CalibrationConfidenceClassTag::Strong => Self::Strong,
            },
        )
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "CalibrationConfidenceRepr")]
pub struct CalibrationConfidence {
    class: CalibrationConfidenceClass,
    sample_count: u32,
    stddev: Option<f32>,
}

impl CalibrationConfidence {
    /// Build a confidence summary from sample count and optional dispersion.
    ///
    /// A missing standard deviation can be at most `Reasonable`; `Strong`
    /// requires enough samples and an observed dispersion value.
    pub fn new(sample_count: u32, stddev: Option<f32>) -> Result<Self, CalibrationError> {
        if sample_count == 0 {
            return Err(CalibrationError::SampleCountIsZero);
        }
        if let Some(value) = stddev
            && (!value.is_finite() || value < 0.0)
        {
            return Err(CalibrationError::InvalidStddev);
        }

        let class = if sample_count >= STRONG_CONFIDENCE_MIN_SAMPLES && stddev.is_some() {
            CalibrationConfidenceClass::Strong
        } else if sample_count >= REASONABLE_CONFIDENCE_MIN_SAMPLES {
            CalibrationConfidenceClass::Reasonable
        } else {
            CalibrationConfidenceClass::Weak
        };

        Ok(Self {
            class,
            sample_count,
            stddev,
        })
    }

    #[must_use]
    pub const fn class(&self) -> CalibrationConfidenceClass {
        self.class
    }

    #[must_use]
    pub const fn sample_count(&self) -> u32 {
        self.sample_count
    }

    #[must_use]
    pub const fn stddev(&self) -> Option<f32> {
        self.stddev
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
struct CalibrationConfidenceRepr {
    #[serde(default)]
    class: Option<CalibrationConfidenceClass>,
    sample_count: u32,
    stddev: Option<f32>,
}

impl TryFrom<CalibrationConfidenceRepr> for CalibrationConfidence {
    type Error = CalibrationError;

    fn try_from(value: CalibrationConfidenceRepr) -> Result<Self, Self::Error> {
        let confidence = Self::new(value.sample_count, value.stddev)?;
        if let Some(class) = value.class
            && class != confidence.class()
        {
            return Err(CalibrationError::ConfidenceClassMismatch {
                expected: confidence.class(),
                actual: class,
            });
        }
        Ok(confidence)
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ValidityEnvelope {
    pub valid_until_compiler_version: SemVer,
    pub valid_until_runtime_nucleus_hash: Option<Hash256>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct UnixTimestampMillis(pub i64);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EmulatorAdapter {
    pub name: String,
    pub version: SemVer,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HardwareIdentifier {
    pub unit_id: String,
    pub cartridge_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MeasurementTarget {
    Emulator(EmulatorAdapter),
    Hardware(HardwareIdentifier),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeasurementContext {
    pub target: MeasurementTarget,
    pub measured_at: UnixTimestampMillis,
    pub gbllm_version: SemVer,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PlatformCalibrationBundleRepr")]
pub struct PlatformCalibrationBundle {
    id: PlatformCalibrationId,
    target_profile: TargetProfileId,
    target_family: TargetFamilyId,
    measurement_context: MeasurementContext,
    bank_switch_cost: CycleDistribution,
    sram_page_cost: CycleDistribution,
    timer_isr_cost: CycleDistribution,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
    cohort: CalibrationCohortId,
}

impl PlatformCalibrationBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        id: PlatformCalibrationId,
        target_profile: TargetProfileId,
        target_family: TargetFamilyId,
        measurement_context: MeasurementContext,
        bank_switch_cost: CycleDistribution,
        sram_page_cost: CycleDistribution,
        timer_isr_cost: CycleDistribution,
        confidence: CalibrationConfidence,
        valid_for: ValidityEnvelope,
        cohort: CalibrationCohortId,
    ) -> Result<Self, CalibrationError> {
        Ok(Self {
            id,
            target_profile,
            target_family,
            measurement_context,
            bank_switch_cost,
            sram_page_cost,
            timer_isr_cost,
            confidence,
            valid_for,
            cohort,
        })
    }

    #[must_use]
    pub fn id(&self) -> &PlatformCalibrationId {
        &self.id
    }

    #[must_use]
    pub fn target_profile(&self) -> &TargetProfileId {
        &self.target_profile
    }

    #[must_use]
    pub fn target_family(&self) -> &TargetFamilyId {
        &self.target_family
    }

    #[must_use]
    pub const fn measurement_context(&self) -> &MeasurementContext {
        &self.measurement_context
    }

    #[must_use]
    pub const fn bank_switch_cost(&self) -> CycleDistribution {
        self.bank_switch_cost
    }

    #[must_use]
    pub const fn sram_page_cost(&self) -> CycleDistribution {
        self.sram_page_cost
    }

    #[must_use]
    pub const fn timer_isr_cost(&self) -> CycleDistribution {
        self.timer_isr_cost
    }

    #[must_use]
    pub const fn confidence(&self) -> CalibrationConfidence {
        self.confidence
    }

    #[must_use]
    pub const fn valid_for(&self) -> ValidityEnvelope {
        self.valid_for
    }

    #[must_use]
    pub fn cohort(&self) -> &CalibrationCohortId {
        &self.cohort
    }
}

#[derive(Clone, Debug, Deserialize)]
struct PlatformCalibrationBundleRepr {
    id: PlatformCalibrationId,
    target_profile: TargetProfileId,
    target_family: TargetFamilyId,
    measurement_context: MeasurementContext,
    bank_switch_cost: CycleDistribution,
    sram_page_cost: CycleDistribution,
    timer_isr_cost: CycleDistribution,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
    cohort: CalibrationCohortId,
}

impl TryFrom<PlatformCalibrationBundleRepr> for PlatformCalibrationBundle {
    type Error = CalibrationError;

    fn try_from(value: PlatformCalibrationBundleRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.id,
            value.target_profile,
            value.target_family,
            value.measurement_context,
            value.bank_switch_cost,
            value.sram_page_cost,
            value.timer_isr_cost,
            value.confidence,
            value.valid_for,
            value.cohort,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "KernelCalibrationBundleRepr")]
pub struct KernelCalibrationBundle {
    id: KernelCalibrationId,
    target_family: TargetFamilyId,
    kernel_impl_hash: Hash256,
    runtime_nucleus_hash: Hash256,
    kernel_profiles: Vec<MeasuredKernelProfile>,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
}

impl KernelCalibrationBundle {
    pub fn try_new(
        id: KernelCalibrationId,
        target_family: TargetFamilyId,
        kernel_impl_hash: Hash256,
        runtime_nucleus_hash: Hash256,
        kernel_profiles: Vec<MeasuredKernelProfile>,
        confidence: CalibrationConfidence,
        valid_for: ValidityEnvelope,
    ) -> Result<Self, CalibrationError> {
        if kernel_profiles.is_empty() {
            return Err(CalibrationError::EmptyKernelProfiles);
        }
        for (index, profile) in kernel_profiles.iter().enumerate() {
            if kernel_profiles[..index].iter().any(|previous| {
                previous.kernel_spec_id() == profile.kernel_spec_id()
                    && previous.tile_dims() == profile.tile_dims()
            }) {
                return Err(CalibrationError::DuplicateKernelProfile);
            }
        }
        if let Some(found) = valid_for.valid_until_runtime_nucleus_hash
            && found != runtime_nucleus_hash
        {
            return Err(CalibrationError::NucleusHashMismatch {
                expected: runtime_nucleus_hash,
                found,
            });
        }

        Ok(Self {
            id,
            target_family,
            kernel_impl_hash,
            runtime_nucleus_hash,
            kernel_profiles,
            confidence,
            valid_for,
        })
    }

    #[must_use]
    pub fn id(&self) -> &KernelCalibrationId {
        &self.id
    }

    #[must_use]
    pub fn target_family(&self) -> &TargetFamilyId {
        &self.target_family
    }

    #[must_use]
    pub const fn kernel_impl_hash(&self) -> Hash256 {
        self.kernel_impl_hash
    }

    #[must_use]
    pub const fn runtime_nucleus_hash(&self) -> Hash256 {
        self.runtime_nucleus_hash
    }

    #[must_use]
    pub fn kernel_profiles(&self) -> &[MeasuredKernelProfile] {
        &self.kernel_profiles
    }

    #[must_use]
    pub const fn confidence(&self) -> CalibrationConfidence {
        self.confidence
    }

    #[must_use]
    pub const fn valid_for(&self) -> ValidityEnvelope {
        self.valid_for
    }
}

#[derive(Clone, Debug, Deserialize)]
struct KernelCalibrationBundleRepr {
    id: KernelCalibrationId,
    target_family: TargetFamilyId,
    kernel_impl_hash: Hash256,
    runtime_nucleus_hash: Hash256,
    kernel_profiles: Vec<MeasuredKernelProfile>,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
}

impl TryFrom<KernelCalibrationBundleRepr> for KernelCalibrationBundle {
    type Error = CalibrationError;

    fn try_from(value: KernelCalibrationBundleRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.id,
            value.target_family,
            value.kernel_impl_hash,
            value.runtime_nucleus_hash,
            value.kernel_profiles,
            value.confidence,
            value.valid_for,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RuntimeCalibrationBundleRepr")]
pub struct RuntimeCalibrationBundle {
    id: RuntimeCalibrationId,
    target_family: TargetFamilyId,
    runtime_nucleus_hash: Hash256,
    scheduler_overhead: CycleDistribution,
    overlay_install_cost: CycleDistribution,
    trace_event_cost: CycleDistribution,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
}

impl RuntimeCalibrationBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        id: RuntimeCalibrationId,
        target_family: TargetFamilyId,
        runtime_nucleus_hash: Hash256,
        scheduler_overhead: CycleDistribution,
        overlay_install_cost: CycleDistribution,
        trace_event_cost: CycleDistribution,
        confidence: CalibrationConfidence,
        valid_for: ValidityEnvelope,
    ) -> Result<Self, CalibrationError> {
        if let Some(found) = valid_for.valid_until_runtime_nucleus_hash
            && found != runtime_nucleus_hash
        {
            return Err(CalibrationError::NucleusHashMismatch {
                expected: runtime_nucleus_hash,
                found,
            });
        }

        Ok(Self {
            id,
            target_family,
            runtime_nucleus_hash,
            scheduler_overhead,
            overlay_install_cost,
            trace_event_cost,
            confidence,
            valid_for,
        })
    }

    #[must_use]
    pub fn id(&self) -> &RuntimeCalibrationId {
        &self.id
    }

    #[must_use]
    pub fn target_family(&self) -> &TargetFamilyId {
        &self.target_family
    }

    #[must_use]
    pub const fn runtime_nucleus_hash(&self) -> Hash256 {
        self.runtime_nucleus_hash
    }

    #[must_use]
    pub const fn scheduler_overhead(&self) -> CycleDistribution {
        self.scheduler_overhead
    }

    #[must_use]
    pub const fn overlay_install_cost(&self) -> CycleDistribution {
        self.overlay_install_cost
    }

    #[must_use]
    pub const fn trace_event_cost(&self) -> CycleDistribution {
        self.trace_event_cost
    }

    #[must_use]
    pub const fn confidence(&self) -> CalibrationConfidence {
        self.confidence
    }

    #[must_use]
    pub const fn valid_for(&self) -> ValidityEnvelope {
        self.valid_for
    }
}

#[derive(Clone, Debug, Deserialize)]
struct RuntimeCalibrationBundleRepr {
    id: RuntimeCalibrationId,
    target_family: TargetFamilyId,
    runtime_nucleus_hash: Hash256,
    scheduler_overhead: CycleDistribution,
    overlay_install_cost: CycleDistribution,
    trace_event_cost: CycleDistribution,
    confidence: CalibrationConfidence,
    valid_for: ValidityEnvelope,
}

impl TryFrom<RuntimeCalibrationBundleRepr> for RuntimeCalibrationBundle {
    type Error = CalibrationError;

    fn try_from(value: RuntimeCalibrationBundleRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.id,
            value.target_family,
            value.runtime_nucleus_hash,
            value.scheduler_overhead,
            value.overlay_install_cost,
            value.trace_event_cost,
            value.confidence,
            value.valid_for,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Default)]
pub struct CalibrationSetRef {
    pub platform: Option<PlatformCalibrationId>,
    pub kernel: Option<KernelCalibrationId>,
    pub runtime: Option<RuntimeCalibrationId>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(try_from = "TileDimsRepr")]
pub struct TileDims {
    rows: u16,
    cols: u16,
}

impl TileDims {
    pub const fn try_new(rows: u16, cols: u16) -> Result<Self, CalibrationError> {
        if rows == 0 || cols == 0 {
            return Err(CalibrationError::InvalidTileDims { rows, cols });
        }

        Ok(Self { rows, cols })
    }

    #[must_use]
    pub const fn rows(self) -> u16 {
        self.rows
    }

    #[must_use]
    pub const fn cols(self) -> u16 {
        self.cols
    }
}

#[derive(Copy, Clone, Debug, Deserialize)]
struct TileDimsRepr {
    rows: u16,
    cols: u16,
}

impl TryFrom<TileDimsRepr> for TileDims {
    type Error = CalibrationError;

    fn try_from(value: TileDimsRepr) -> Result<Self, Self::Error> {
        Self::try_new(value.rows, value.cols)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MeasuredKernelProfileRepr")]
pub struct MeasuredKernelProfile {
    kernel_spec_id: KernelSpecId,
    tile_dims: TileDims,
    cycles: CycleDistribution,
    bytes_in: u32,
    bytes_out: u32,
}

impl MeasuredKernelProfile {
    pub fn try_new(
        kernel_spec_id: KernelSpecId,
        tile_dims: TileDims,
        cycles: CycleDistribution,
        bytes_in: u32,
        bytes_out: u32,
    ) -> Result<Self, CalibrationError> {
        if bytes_in == 0 || bytes_out == 0 {
            return Err(CalibrationError::InvalidKernelProfileBytes {
                bytes_in,
                bytes_out,
            });
        }

        Ok(Self {
            kernel_spec_id,
            tile_dims,
            cycles,
            bytes_in,
            bytes_out,
        })
    }

    #[must_use]
    pub fn kernel_spec_id(&self) -> &KernelSpecId {
        &self.kernel_spec_id
    }

    #[must_use]
    pub const fn tile_dims(&self) -> TileDims {
        self.tile_dims
    }

    #[must_use]
    pub const fn cycles(&self) -> CycleDistribution {
        self.cycles
    }

    #[must_use]
    pub const fn bytes_in(&self) -> u32 {
        self.bytes_in
    }

    #[must_use]
    pub const fn bytes_out(&self) -> u32 {
        self.bytes_out
    }
}

#[derive(Clone, Debug, Deserialize)]
struct MeasuredKernelProfileRepr {
    kernel_spec_id: KernelSpecId,
    tile_dims: TileDims,
    cycles: CycleDistribution,
    bytes_in: u32,
    bytes_out: u32,
}

impl TryFrom<MeasuredKernelProfileRepr> for MeasuredKernelProfile {
    type Error = CalibrationError;

    fn try_from(value: MeasuredKernelProfileRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.kernel_spec_id,
            value.tile_dims,
            value.cycles,
            value.bytes_in,
            value.bytes_out,
        )
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum CalibrationError {
    SampleCountIsZero,
    InvalidCycleDistribution,
    InvalidStddev,
    StaleEnvelope {
        current_version: SemVer,
        valid_until: SemVer,
    },
    NucleusHashMismatch {
        expected: Hash256,
        found: Hash256,
    },
    EmptyKernelProfiles,
    DuplicateKernelProfile,
    ConfidenceClassMismatch {
        expected: CalibrationConfidenceClass,
        actual: CalibrationConfidenceClass,
    },
    InvalidTileDims {
        rows: u16,
        cols: u16,
    },
    InvalidKernelProfileBytes {
        bytes_in: u32,
        bytes_out: u32,
    },
}

impl fmt::Display for CalibrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SampleCountIsZero => {
                f.write_str("calibration confidence needs at least one sample")
            }
            Self::InvalidCycleDistribution => {
                f.write_str("cycle distribution must be finite, non-negative, and monotone")
            }
            Self::InvalidStddev => f.write_str("stddev must be finite and non-negative"),
            Self::StaleEnvelope {
                current_version,
                valid_until,
            } => write!(
                f,
                "calibration envelope is stale for compiler {current_version}; valid until {valid_until}"
            ),
            Self::NucleusHashMismatch { expected, found } => {
                write!(
                    f,
                    "runtime nucleus hash mismatch: expected {expected}, found {found}"
                )
            }
            Self::EmptyKernelProfiles => {
                f.write_str("kernel calibration bundle requires at least one profile")
            }
            Self::DuplicateKernelProfile => {
                f.write_str("kernel calibration bundle contains duplicate kernel/tile profile")
            }
            Self::ConfidenceClassMismatch { expected, actual } => {
                write!(
                    f,
                    "confidence class mismatch: expected {expected:?}, got {actual:?}"
                )
            }
            Self::InvalidTileDims { rows, cols } => {
                write!(f, "tile dimensions must be non-zero, got {rows}x{cols}")
            }
            Self::InvalidKernelProfileBytes {
                bytes_in,
                bytes_out,
            } => write!(
                f,
                "kernel profile byte counts must be non-zero, got in={bytes_in}, out={bytes_out}"
            ),
        }
    }
}

impl std::error::Error for CalibrationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn dist() -> CycleDistribution {
        CycleDistribution::new(12.5, 10, 13, 20).unwrap()
    }

    fn confidence() -> CalibrationConfidence {
        CalibrationConfidence::new(1_000, Some(0.5)).unwrap()
    }

    fn platform_valid_for() -> ValidityEnvelope {
        ValidityEnvelope {
            valid_until_compiler_version: SemVer::new(0, 1, 0),
            valid_until_runtime_nucleus_hash: None,
        }
    }

    fn runtime_valid_for(runtime_nucleus_hash: Hash256) -> ValidityEnvelope {
        ValidityEnvelope {
            valid_until_compiler_version: SemVer::new(0, 1, 0),
            valid_until_runtime_nucleus_hash: Some(runtime_nucleus_hash),
        }
    }

    fn context() -> MeasurementContext {
        MeasurementContext {
            target: MeasurementTarget::Emulator(EmulatorAdapter {
                name: "gameroy".to_owned(),
                version: SemVer::new(0, 1, 0),
            }),
            measured_at: UnixTimestampMillis(1_714_000_000_000),
            gbllm_version: SemVer::new(0, 1, 0),
        }
    }

    fn measured_kernel_profile() -> MeasuredKernelProfile {
        MeasuredKernelProfile::try_new(
            KernelSpecId::from("dense-u8"),
            TileDims::try_new(8, 16).unwrap(),
            dist(),
            128,
            64,
        )
        .unwrap()
    }

    #[test]
    fn serde_round_trip() {
        let platform = PlatformCalibrationBundle::try_new(
            PlatformCalibrationId::from("platform"),
            TargetProfileId::from("dmg"),
            TargetFamilyId::from("dmg-family"),
            context(),
            dist(),
            dist(),
            dist(),
            confidence(),
            platform_valid_for(),
            CalibrationCohortId::from("cohort"),
        )
        .unwrap();
        let runtime = RuntimeCalibrationBundle::try_new(
            RuntimeCalibrationId::from("runtime"),
            TargetFamilyId::from("dmg-family"),
            Hash256::ZERO,
            dist(),
            dist(),
            dist(),
            confidence(),
            runtime_valid_for(Hash256::ZERO),
        )
        .unwrap();
        let kernel = KernelCalibrationBundle::try_new(
            KernelCalibrationId::from("kernel"),
            TargetFamilyId::from("dmg-family"),
            Hash256::ZERO,
            Hash256::ZERO,
            vec![measured_kernel_profile()],
            confidence(),
            runtime_valid_for(Hash256::ZERO),
        )
        .unwrap();

        let platform_json = serde_json::to_string(&platform).unwrap();
        let runtime_json = serde_json::to_string(&runtime).unwrap();
        let kernel_json = serde_json::to_string(&kernel).unwrap();

        let platform_value = serde_json::to_value(&platform).unwrap();
        assert_eq!(platform_value["id"], serde_json::json!("platform"));
        assert_eq!(platform_value["target_profile"], serde_json::json!("dmg"));
        assert_eq!(
            platform_value["target_family"],
            serde_json::json!("dmg-family")
        );
        assert_eq!(
            platform_value["valid_for"]["valid_until_runtime_nucleus_hash"],
            serde_json::Value::Null
        );
        assert_eq!(
            serde_json::to_value(measured_kernel_profile()).unwrap(),
            serde_json::json!({
                "kernel_spec_id": "dense-u8",
                "tile_dims": {"rows": 8, "cols": 16},
                "cycles": {"mean": 12.5, "p50": 10, "p90": 13, "p99": 20},
                "bytes_in": 128,
                "bytes_out": 64
            })
        );

        assert_eq!(
            serde_json::from_str::<PlatformCalibrationBundle>(&platform_json).unwrap(),
            platform
        );
        assert_eq!(
            serde_json::from_str::<RuntimeCalibrationBundle>(&runtime_json).unwrap(),
            runtime
        );
        assert_eq!(
            serde_json::from_str::<KernelCalibrationBundle>(&kernel_json).unwrap(),
            kernel
        );
    }

    #[test]
    fn confidence_class_ordering() {
        assert!(CalibrationConfidenceClass::Strong > CalibrationConfidenceClass::Reasonable);
        assert!(CalibrationConfidenceClass::Reasonable > CalibrationConfidenceClass::Weak);
    }

    #[test]
    fn set_ref_all_layers_optional() {
        assert_eq!(
            CalibrationSetRef::default(),
            CalibrationSetRef {
                platform: None,
                kernel: None,
                runtime: None,
            }
        );
    }

    #[test]
    fn confidence_rejects_zero_samples() {
        assert_eq!(
            CalibrationConfidence::new(0, None),
            Err(CalibrationError::SampleCountIsZero)
        );
    }

    #[test]
    fn confidence_class_is_derived() {
        assert_eq!(
            CalibrationConfidence::new(1, Some(0.1)).unwrap().class(),
            CalibrationConfidenceClass::Weak
        );
        assert_eq!(
            CalibrationConfidence::new(100, None).unwrap().class(),
            CalibrationConfidenceClass::Reasonable
        );
        assert_eq!(
            CalibrationConfidence::new(1_000, None).unwrap().class(),
            CalibrationConfidenceClass::Reasonable
        );
        assert_eq!(
            CalibrationConfidence::new(1_000, Some(100.0))
                .unwrap()
                .class(),
            CalibrationConfidenceClass::Strong
        );
        assert_eq!(
            CalibrationConfidence::new(1_000, Some(0.1))
                .unwrap()
                .class(),
            CalibrationConfidenceClass::Strong
        );
    }

    #[test]
    fn cycle_distribution_rejects_nan_mean() {
        assert_eq!(
            CycleDistribution::new(f32::NAN, 1, 2, 3),
            Err(CalibrationError::InvalidCycleDistribution)
        );
    }

    #[test]
    fn cycle_distribution_rejects_negative_mean() {
        assert_eq!(
            CycleDistribution::new(-1.0, 1, 2, 3),
            Err(CalibrationError::InvalidCycleDistribution)
        );
    }

    #[test]
    fn cycle_distribution_rejects_non_monotone() {
        assert_eq!(
            CycleDistribution::new(1.0, 3, 2, 4),
            Err(CalibrationError::InvalidCycleDistribution)
        );
    }

    #[test]
    fn stddev_rejects_nan_or_negative() {
        assert_eq!(
            CalibrationConfidence::new(100, Some(f32::NAN)),
            Err(CalibrationError::InvalidStddev)
        );
        assert_eq!(
            CalibrationConfidence::new(100, Some(-0.1)),
            Err(CalibrationError::InvalidStddev)
        );
    }

    #[test]
    fn kernel_bundle_rejects_empty_profiles() {
        assert_eq!(
            KernelCalibrationBundle::try_new(
                KernelCalibrationId::from("kernel"),
                TargetFamilyId::from("dmg"),
                Hash256::ZERO,
                Hash256::ZERO,
                Vec::new(),
                confidence(),
                runtime_valid_for(Hash256::ZERO),
            ),
            Err(CalibrationError::EmptyKernelProfiles)
        );
    }

    #[test]
    fn kernel_bundle_rejects_duplicate_profiles() {
        assert_eq!(
            KernelCalibrationBundle::try_new(
                KernelCalibrationId::from("kernel"),
                TargetFamilyId::from("dmg"),
                Hash256::ZERO,
                Hash256::ZERO,
                vec![measured_kernel_profile(), measured_kernel_profile()],
                confidence(),
                runtime_valid_for(Hash256::ZERO),
            ),
            Err(CalibrationError::DuplicateKernelProfile)
        );
    }

    #[test]
    fn serde_runs_validation() {
        let bad_dist = r#"{"mean":-1.0,"p50":3,"p90":2,"p99":4}"#;
        assert!(serde_json::from_str::<CycleDistribution>(bad_dist).is_err());

        let bad_confidence = r#"{"sample_count":0,"stddev":null}"#;
        assert!(serde_json::from_str::<CalibrationConfidence>(bad_confidence).is_err());

        let bad_confidence_class = r#"{"class":"Strong","sample_count":100,"stddev":0.1}"#;
        assert!(serde_json::from_str::<CalibrationConfidence>(bad_confidence_class).is_err());

        let bad_tile_dims = r#"{"rows":0,"cols":16}"#;
        assert!(serde_json::from_str::<TileDims>(bad_tile_dims).is_err());

        let bad_profile = r#"{
            "kernel_spec_id":"dense-u8",
            "tile_dims":{"rows":8,"cols":16},
            "cycles":{"mean":12.5,"p50":10,"p90":13,"p99":20},
            "bytes_in":0,
            "bytes_out":64
        }"#;
        assert!(serde_json::from_str::<MeasuredKernelProfile>(bad_profile).is_err());

        let bad_kernel = r#"{
            "id":"kernel",
            "target_family":"dmg",
            "kernel_impl_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "runtime_nucleus_hash":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0],
            "kernel_profiles":[],
            "confidence":{"sample_count":1000,"stddev":0.1},
            "valid_for":{"valid_until_compiler_version":{"major":0,"minor":1,"patch":0},"valid_until_runtime_nucleus_hash":null}
        }"#;
        assert!(serde_json::from_str::<KernelCalibrationBundle>(bad_kernel).is_err());
    }

    #[test]
    fn runtime_and_kernel_bundles_reject_mismatched_runtime_hash() {
        let found = Hash256::from_bytes([1; 32]);
        assert_eq!(
            RuntimeCalibrationBundle::try_new(
                RuntimeCalibrationId::from("runtime"),
                TargetFamilyId::from("dmg-family"),
                Hash256::ZERO,
                dist(),
                dist(),
                dist(),
                confidence(),
                runtime_valid_for(found),
            ),
            Err(CalibrationError::NucleusHashMismatch {
                expected: Hash256::ZERO,
                found,
            })
        );

        assert_eq!(
            KernelCalibrationBundle::try_new(
                KernelCalibrationId::from("kernel"),
                TargetFamilyId::from("dmg-family"),
                Hash256::ZERO,
                Hash256::ZERO,
                vec![measured_kernel_profile()],
                confidence(),
                runtime_valid_for(found),
            ),
            Err(CalibrationError::NucleusHashMismatch {
                expected: Hash256::ZERO,
                found,
            })
        );
    }

    #[test]
    fn bundle_id_is_content_addressed_by_value_identity() {
        let left = CalibrationSetRef {
            platform: Some(PlatformCalibrationId::from("platform")),
            kernel: Some(KernelCalibrationId::from("kernel")),
            runtime: Some(RuntimeCalibrationId::from("runtime")),
        };
        let right = left.clone();
        assert_eq!(left, right);
    }
}
