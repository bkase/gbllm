//! Schedule cost-analysis contracts.

use gbf_foundation::{EvidenceRef, Hash256, SemVer};
use serde::{Deserialize, Serialize};

use crate::compile::RuntimeMode;
use crate::objective::CompileObjective;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    Calibrated,
    Transferred,
    Heuristic,
    Fallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UncertaintyEnvelope {
    pub p50_q16_16: i64,
    pub p95_lower_q16_16: i64,
    pub p95_upper_q16_16: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p99_upper_q16_16: Option<i64>,
}

impl UncertaintyEnvelope {
    pub const Q16_ONE: i64 = 65_536;

    #[must_use]
    pub const fn exact_units(units: i64) -> Self {
        let q16 = units.saturating_mul(Self::Q16_ONE);
        Self {
            p50_q16_16: q16,
            p95_lower_q16_16: q16,
            p95_upper_q16_16: q16,
            p99_upper_q16_16: Some(q16),
        }
    }

    #[must_use]
    pub const fn from_q16(
        p50: i64,
        p95_lower: i64,
        p95_upper: i64,
        p99_upper: Option<i64>,
    ) -> Self {
        Self {
            p50_q16_16: p50,
            p95_lower_q16_16: p95_lower,
            p95_upper_q16_16: p95_upper,
            p99_upper_q16_16: p99_upper,
        }
    }

    #[must_use]
    pub const fn is_ordered(self) -> bool {
        if self.p95_lower_q16_16 > self.p50_q16_16 || self.p50_q16_16 > self.p95_upper_q16_16 {
            return false;
        }
        match self.p99_upper_q16_16 {
            Some(p99) => self.p95_upper_q16_16 <= p99,
            None => true,
        }
    }

    #[must_use]
    pub const fn is_non_negative(self) -> bool {
        self.p95_lower_q16_16 >= 0
            && self.p50_q16_16 >= 0
            && self.p95_upper_q16_16 >= 0
            && match self.p99_upper_q16_16 {
                Some(p99) => p99 >= 0,
                None => true,
            }
    }

    #[must_use]
    pub const fn upper_for(self, quantile: Quantile) -> i64 {
        match quantile {
            Quantile::P50 => self.p50_q16_16,
            Quantile::P95 => self.p95_upper_q16_16,
            Quantile::P99 => match self.p99_upper_q16_16 {
                Some(p99) => p99,
                None => self.p95_upper_q16_16,
            },
        }
    }

    #[must_use]
    pub const fn lower_for(self, quantile: Quantile) -> i64 {
        match quantile {
            Quantile::P50 => self.p50_q16_16,
            Quantile::P95 | Quantile::P99 => self.p95_lower_q16_16,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostEstimate {
    pub evidence_class: EvidenceClass,
    pub envelope: UncertaintyEnvelope,
    pub refs: Vec<EvidenceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback_reason: Option<FallbackReason>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "fields", deny_unknown_fields)]
pub enum FallbackReason {
    NoBundleForTarget,
    ConfidenceBelowRequirement {
        declared: String,
        required: String,
    },
    KernelSpecNotCalibrated,
    BundleStale {
        field: StaleCalibrationField,
        declared: Hash256,
        observed: Hash256,
    },
    MeasurementShapeMismatch,
    UpstreamFallback {
        reason: Box<FallbackReason>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum StaleCalibrationField {
    TargetProfileHash,
    KernelSetHash,
    PackerVersion,
    CalibrationSchemaHash,
    ValidityEnvelope,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EstimatedCostDelta {
    pub cycles_per_token: CostEstimate,
    pub bank_switches_per_token: CostEstimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sram_page_switches_per_token: Option<CostEstimate>,
    pub yields_per_token: CostEstimate,
    pub scheduler_headroom_utilization: CostEstimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_commit_cost_margin: Option<CostEstimate>,
    pub max_no_progress_estimate: CostEstimate,
    pub time_to_first_token: CostEstimate,
    pub sustained_throughput_tokens_per_megacycle: CostEstimate,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame_jitter: Option<CostEstimate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveAxis {
    CyclesPerToken,
    BankSwitchesPerToken,
    SramPageSwitchesPerToken,
    SchedulerHeadroomUtilization,
    TimeToFirstToken,
    SustainedThroughputTokensPerMegacycle,
    FrameJitter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quantile {
    P50,
    P95,
    P99,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveSatisfaction {
    Satisfied,
    Borderline,
    Violated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SatisfactionKey {
    pub mode: RuntimeMode,
    pub axis: ObjectiveAxis,
    pub quantile: Quantile,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSatisfactionMatrix {
    pub entries: Vec<SatisfactionEntry>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SatisfactionEntry {
    pub key: SatisfactionKey,
    pub satisfaction: ObjectiveSatisfaction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostIdentity {
    pub schedule_pack_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub calibration_bundle_set_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub kernel_spec_registry_hash: Hash256,
    pub schedule_cost_policy_projection_hash: Hash256,
    pub pass_version: SemVer,
    pub crate_feature_set_hash: Hash256,
    pub schedule_cost_schema_hash: Hash256,
    pub schedule_cost_report_self_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostReport {
    pub objective: CompileObjective,
    pub per_mode: Vec<ModeEstimatedCost>,
    pub satisfaction: ObjectiveSatisfactionMatrix,
    pub refs: Vec<EvidenceRef>,
    pub identity: ScheduleCostIdentity,
    pub breakdown: ScheduleCostBreakdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeEstimatedCost {
    pub mode: RuntimeMode,
    pub delta: EstimatedCostDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleCostBreakdown {
    pub per_mode: Vec<ModeCostBreakdownEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeCostBreakdownEntry {
    pub mode: RuntimeMode,
    pub breakdown: ModeCostBreakdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModeCostBreakdown {
    pub slices: Vec<SliceCostBreakdown>,
    pub totals: CostBucketTotals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SliceCostBreakdown {
    pub slice_id: u32,
    pub bank_switch_cycles: u64,
    pub sram_page_switch_cycles: u64,
    pub overlay_install_cycles: u64,
    pub static_slice_cycles: u64,
    pub total_cycles: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostBucketTotals {
    pub bank_switch_cycles: u64,
    pub sram_page_switch_cycles: u64,
    pub overlay_install_cycles: u64,
    pub static_slice_cycles: u64,
    pub total_cycles: u64,
    pub bank_switches: u64,
    pub sram_page_switches: u64,
    pub overlay_installs: u64,
    pub yields: u64,
}
