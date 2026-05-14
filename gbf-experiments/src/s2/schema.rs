//! Canonical S2 notation types.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;

use gbf_foundation::{Hash256, SemVer};
use serde::{Deserialize, Deserializer, Serialize};

use crate::S2_LOG_TARGET;
use crate::s1::device_profile::S1CpuDeterministic;
use crate::s1::run::{
    AdamWConfig, RngKind, S1_BATCH_SIZE, S1_EVAL_EVERY_STEPS, S1_EVAL_SUBSET_SIZE,
    S1_OPTIMIZER_STEPS, S1_SEQUENCE_LENGTH,
};
use crate::s1::schema::{
    DomainHash, GitCommitId, RfcRevisionRef, S1CanonicalJson, S1SchemaError, self_hash_for_value,
};
use crate::s2::environment::S2EnvironmentHash;

pub use gbf_model::qat::QuantHardness;

/// Global 1-indexed optimizer step counter across the S2 run.
pub type GlobalStep = u64;

/// 1-indexed step counter relative to the current S2 phase.
pub type PhaseLocalStep = u64;

/// Step counter used by phase artifacts.
pub type PhaseStep = GlobalStep;

/// Distillation temperature scalar.
pub type DistillTemperature = f32;

/// Distillation loss in nats for reported f32 values.
pub type DistillLossNats = f32;

/// Distillation loss in nats for promoted accumulators.
pub type DistillLossNats64 = f64;

/// Bits-per-byte gap, computed as ternary minus fp.
pub type GapBpc = f64;

/// Q8.8 scale payload.
pub type Q8_8Scale = u16;

/// S2 optimizer steps inherited from S1 and repartitioned by D1.
pub const S2_OPTIMIZER_STEPS: u64 = S1_OPTIMIZER_STEPS;

/// End of Phase A and frozen-teacher checkpoint step.
pub const S2_TEACHER_FREEZE_STEP: u64 = 4_000;

/// End of Phase B / router warmup.
pub const S2_PHASE_B_END_STEP: u64 = 5_000;

/// End of Phase C / expert ternary QAT.
pub const S2_PHASE_C_END_STEP: u64 = 8_000;

/// Default S2 distillation temperature from D3.
pub const S2_DISTILL_TEMPERATURE: f32 = 2.0;

/// Default S2 distillation lambda for full ternary/fp builds.
pub const S2_LAMBDA_DISTILL_DEFAULT: f32 = 1.0;

/// Default S2 range-loss lambda.
pub const S2_LAMBDA_RANGE: f32 = 0.01;

/// Default S2 zero-loss lambda.
pub const S2_LAMBDA_ZERO: f32 = 0.0001;

/// Default S2 activation lower safe bound.
pub const S2_RANGE_SAFE_LO: f32 = -1.0;

/// Default S2 activation upper safe bound.
pub const S2_RANGE_SAFE_HI: f32 = 1.0;

/// Default deterministic threshold initialization multiplier.
pub const S2_THRESHOLD_INIT_MULTIPLIER: f32 = 0.7;

const LOSS_GRAD_FLOW_SCHEMA: &str = "s2_loss_grad_flow.v1";
const LINEARSTATE_SMOKE_SCHEMA: &str = "s2_linearstate_grad_smoke.v1";
const PHASE_TRANSITION_INTEG_SCHEMA: &str = "s2_phase_transition_integration.v1";
const PHASE_LOG_SCHEMA: &str = "s2_phase_log.v1";
const S2_SCORE_SCHEMA: &str = "s2_score.v1";
const DISTILLATION_LOG_SCHEMA: &str = "s2_distillation_log.v1";
const S2_ABLATION_SCHEMA: &str = "s2_ablation.v1";
const S2_ORACLE_RE_RUN_SCHEMA: &str = "s2_oracle_re_run.v1";
const S2_REPORT_SCHEMA: &str = "s2_report.v1";
const STOP_GRAD_EPS: f32 = 1.0e-6;
const PHASE_LOG_HASH_DOMAIN_SEPARATOR: &[u8] = b"s2_phase_log.v1/header+entries\0";

/// S2 production phase kind. Phase E is outside the S2 run protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseKindS2 {
    /// Dense teacher warmup, inherited from S1 Phase A semantics.
    PhaseA,
    /// Router warmup phase; Toy0 carries no router.
    PhaseB,
    /// Expert ternary QAT phase.
    PhaseC,
    /// Full numeric QAT phase.
    PhaseD,
}

/// Fixture phase kind that includes Phase E for scheduler integration tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PhaseKindFixture {
    /// Dense teacher warmup.
    PhaseA,
    /// Router warmup phase.
    PhaseB,
    /// Expert ternary QAT phase.
    PhaseC,
    /// Full numeric QAT phase.
    PhaseD,
    /// Harden-and-select fixture phase.
    PhaseE,
}

/// Router training mode vocabulary used by S2 logs and schemas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RouterTrainMode {
    /// Dense models with no router.
    NoRouter,
    /// Soft top-1 router training.
    SoftTop1,
    /// Hard top-1 router training.
    HardTop1,
}

/// Per-component QAT hardness selected for a phase step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HardnessTriple {
    /// Expert weight QAT hardness.
    pub expert_qat: QuantHardness,
    /// Activation fake-quant hardness.
    pub activation_qat: QuantHardness,
    /// Normalization approximation hardness.
    pub norm_qat: QuantHardness,
}

impl HardnessTriple {
    /// Construct a hardness triple.
    #[must_use]
    pub const fn new(
        expert_qat: QuantHardness,
        activation_qat: QuantHardness,
        norm_qat: QuantHardness,
    ) -> Self {
        Self {
            expert_qat,
            activation_qat,
            norm_qat,
        }
    }

    /// Fully disabled QAT hardness used by the runtime fp comparator.
    #[must_use]
    pub const fn all_off() -> Self {
        Self {
            expert_qat: QuantHardness::Off,
            activation_qat: QuantHardness::Off,
            norm_qat: QuantHardness::Off,
        }
    }
}

/// D8 boundary projection input for one live scheduler transition.
///
/// D8 records the boundary-observed ramp state, not the raw static
/// `TrainPhaseSpec` controls. The projection below is the executable owner for
/// that fixture contract: expert Off->Hard enters through Soft at the boundary,
/// while activation/norm changes become visible at the following boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PhaseBoundaryHardnessProjection {
    /// Boundary step that emitted the transition.
    pub step: u64,
    /// Static phase controls before the transition.
    pub from: HardnessTriple,
    /// Static phase controls after the transition.
    pub to: HardnessTriple,
}

impl PhaseBoundaryHardnessProjection {
    /// Construct a boundary projection input.
    #[must_use]
    pub const fn new(step: u64, from: HardnessTriple, to: HardnessTriple) -> Self {
        Self { step, from, to }
    }

    /// Project static scheduler controls into the D8 fixture's boundary row.
    #[must_use]
    pub const fn projected(self) -> HardnessTriple {
        HardnessTriple::new(
            project_expert_boundary(self.from.expert_qat, self.to.expert_qat),
            project_delayed_component_boundary(self.from.activation_qat, self.to.activation_qat),
            project_delayed_component_boundary(self.from.norm_qat, self.to.norm_qat),
        )
    }
}

const fn project_expert_boundary(from: QuantHardness, to: QuantHardness) -> QuantHardness {
    match (from, to) {
        (QuantHardness::Off, QuantHardness::Hard) => QuantHardness::Soft,
        (_, to) => to,
    }
}

const fn project_delayed_component_boundary(
    from: QuantHardness,
    to: QuantHardness,
) -> QuantHardness {
    if same_hardness(from, to) { to } else { from }
}

const fn same_hardness(lhs: QuantHardness, rhs: QuantHardness) -> bool {
    matches!(
        (lhs, rhs),
        (QuantHardness::Off, QuantHardness::Off)
            | (QuantHardness::Soft, QuantHardness::Soft)
            | (QuantHardness::Hard, QuantHardness::Hard)
    )
}

/// Runtime override selected by S2BuildKind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuantHardnessOverride {
    /// Use the scheduled hardness triple unchanged.
    None,
    /// Force expert, activation, and norm QAT hardness to Off.
    AllOff,
}

impl QuantHardnessOverride {
    /// Apply this override to a scheduled hardness triple.
    #[must_use]
    pub const fn apply(self, scheduled: HardnessTriple) -> HardnessTriple {
        match self {
            Self::None => scheduled,
            Self::AllOff => HardnessTriple::all_off(),
        }
    }
}

/// Phase-effective S2 loss weights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseEffectiveLambda {
    /// Effective distillation loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_distill: f32,
    /// Effective router load-balance loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_balance: f32,
    /// Effective router z-loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_zrouter: f32,
    /// Effective switch/router transition loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_switch: f32,
    /// Effective activation/range loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_range: f32,
    /// Effective ternary zero/sparsity loss weight.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_zero: f32,
    /// Effective shape loss weight, carried inert in S2.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_shape: f32,
    /// Effective overflow loss weight, carried inert in S2.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_overflow: f32,
}

impl PhaseEffectiveLambda {
    /// Construct phase-effective loss weights after finite non-negative checks.
    pub fn new(values: PhaseEffectiveLambdaValues) -> Result<Self, S2SchemaError> {
        let lambdas = Self {
            lambda_distill: values.lambda_distill,
            lambda_balance: values.lambda_balance,
            lambda_zrouter: values.lambda_zrouter,
            lambda_switch: values.lambda_switch,
            lambda_range: values.lambda_range,
            lambda_zero: values.lambda_zero,
            lambda_shape: values.lambda_shape,
            lambda_overflow: values.lambda_overflow,
        };
        lambdas.validate()?;
        Ok(lambdas)
    }

    /// Phase C/D default values from F-S2 D3 and D5.
    #[must_use]
    pub const fn phase_cd_defaults() -> Self {
        Self {
            lambda_distill: 1.0,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range: 0.01,
            lambda_zero: 0.0001,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        }
    }

    /// Validate all weights are finite and non-negative.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        for (field, value) in [
            ("lambda_distill", self.lambda_distill),
            ("lambda_balance", self.lambda_balance),
            ("lambda_zrouter", self.lambda_zrouter),
            ("lambda_switch", self.lambda_switch),
            ("lambda_range", self.lambda_range),
            ("lambda_zero", self.lambda_zero),
            ("lambda_shape", self.lambda_shape),
            ("lambda_overflow", self.lambda_overflow),
        ] {
            validate_finite_nonnegative(field, value)?;
        }
        Ok(())
    }
}

/// Input bag for constructing phase-effective loss weights.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseEffectiveLambdaValues {
    /// Distillation loss weight.
    pub lambda_distill: f32,
    /// Router load-balance loss weight.
    pub lambda_balance: f32,
    /// Router z-loss weight.
    pub lambda_zrouter: f32,
    /// Switch/router transition loss weight.
    pub lambda_switch: f32,
    /// Activation/range loss weight.
    pub lambda_range: f32,
    /// Ternary zero/sparsity loss weight.
    pub lambda_zero: f32,
    /// Shape loss weight.
    pub lambda_shape: f32,
    /// Overflow loss weight.
    pub lambda_overflow: f32,
}

/// S2 runtime build identity.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum S2BuildKind {
    /// Ternary A-to-D run with distillation.
    s2_ternary_full,
    /// Full-precision matched-protocol comparator.
    s2_fp_full,
    /// Ternary A-to-D run with distillation disabled.
    s2_ternary_nodistill,
    /// QAT-codepath ablation build.
    s2_ablation,
}

/// Unit used for S2 training loss values and hashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrainingLossUnit {
    /// Natural-log nats, matching cross-entropy and distillation KL.
    Nats,
}

/// Canonical S2 train phase interval, using inclusive 1-indexed global steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainConfigS2Phase {
    /// Phase kind.
    pub phase: PhaseKindS2,
    /// Inclusive first global step in the phase.
    pub start_step: GlobalStep,
    /// Inclusive final global step in the phase.
    pub end_step: GlobalStep,
}

impl TrainConfigS2Phase {
    /// Construct an inclusive phase interval.
    pub fn new(
        phase: PhaseKindS2,
        start_step: GlobalStep,
        end_step: GlobalStep,
    ) -> Result<Self, S2SchemaError> {
        if start_step == 0 || start_step > end_step {
            return Err(S2SchemaError::InvalidPhasePlan(
                "S2 phase ranges are 1-indexed inclusive and non-empty",
            ));
        }
        Ok(Self {
            phase,
            start_step,
            end_step,
        })
    }
}

/// Canonical S2 hardness-ramp identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum HardnessRampS2 {
    /// Phase C and Phase D ramps exactly as pinned by D2.
    PhaseCRampD2PlusPhaseDRampD2,
}

/// Complete A-to-D S2 training configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainConfigS2Full {
    /// Number of optimizer steps.
    pub optimizer_steps: u64,
    /// Number of sampled sequences per optimizer step.
    pub batch_size: usize,
    /// Number of bytes per sampled sequence.
    pub sequence_length: usize,
    /// Progress evaluation cadence.
    pub eval_every_steps: u64,
    /// Progress evaluation subset size in sequences.
    pub eval_subset_size: u64,
    /// AdamW optimizer configuration.
    pub optimizer: AdamWConfig,
    /// Full S2 A-to-D phase plan.
    pub phase_plan: Vec<TrainConfigS2Phase>,
    /// D2 hardness ramp identity.
    pub hardness_ramp: HardnessRampS2,
    /// Distillation temperature.
    pub distill_temp: DistillTemperature,
    /// Default distillation lambda before build-kind override.
    pub lambda_distill_default: f32,
    /// Range-loss lambda.
    pub lambda_range: f32,
    /// Zero-loss lambda.
    pub lambda_zero: f32,
    /// Activation lower safe bound.
    pub range_safe_lo: f32,
    /// Activation upper safe bound.
    pub range_safe_hi: f32,
    /// Ternary threshold initialization multiplier.
    pub threshold_init_multiplier: f32,
    /// Phase A teacher freeze step.
    pub teacher_freeze_step: GlobalStep,
    /// Deterministic RNG kind.
    pub rng_kind: RngKind,
    /// Required deterministic CPU device profile.
    pub device_profile: S1CpuDeterministic,
    /// Explicit training loss unit.
    pub training_loss_unit: TrainingLossUnit,
}

impl TrainConfigS2Full {
    /// Return the RFC-pinned full S2 config.
    #[must_use]
    pub fn pinned() -> Self {
        let config = Self {
            optimizer_steps: S2_OPTIMIZER_STEPS,
            batch_size: S1_BATCH_SIZE,
            sequence_length: S1_SEQUENCE_LENGTH,
            eval_every_steps: S1_EVAL_EVERY_STEPS,
            eval_subset_size: S1_EVAL_SUBSET_SIZE,
            optimizer: AdamWConfig::pinned(),
            phase_plan: full_phase_plan(),
            hardness_ramp: HardnessRampS2::PhaseCRampD2PlusPhaseDRampD2,
            distill_temp: S2_DISTILL_TEMPERATURE,
            lambda_distill_default: S2_LAMBDA_DISTILL_DEFAULT,
            lambda_range: S2_LAMBDA_RANGE,
            lambda_zero: S2_LAMBDA_ZERO,
            range_safe_lo: S2_RANGE_SAFE_LO,
            range_safe_hi: S2_RANGE_SAFE_HI,
            threshold_init_multiplier: S2_THRESHOLD_INIT_MULTIPLIER,
            teacher_freeze_step: S2_TEACHER_FREEZE_STEP,
            rng_kind: RngKind::Pcg64Mcg,
            device_profile: S1CpuDeterministic::canonical(),
            training_loss_unit: TrainingLossUnit::Nats,
        };
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "train_config_defaults_built",
            training_loss_unit = "nats",
            "s2 train config built"
        );
        config
    }

    /// Validate finite scalar fields and canonical phase shape.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        validate_finite_positive("distill_temp", self.distill_temp)?;
        if (self.distill_temp - S2_DISTILL_TEMPERATURE).abs() > f32::EPSILON {
            return Err(S2SchemaError::InvalidPhasePlan(
                "TrainConfigS2Full distill_temp must be pinned to 2.0",
            ));
        }
        validate_finite_nonnegative("lambda_distill_default", self.lambda_distill_default)?;
        validate_finite_nonnegative("lambda_range", self.lambda_range)?;
        validate_finite_nonnegative("lambda_zero", self.lambda_zero)?;
        validate_finite("range_safe_lo", self.range_safe_lo)?;
        validate_finite("range_safe_hi", self.range_safe_hi)?;
        if self.range_safe_lo > self.range_safe_hi {
            return Err(S2SchemaError::InvalidRangeBounds {
                lo: self.range_safe_lo,
                hi: self.range_safe_hi,
            });
        }
        validate_finite_nonnegative("threshold_init_multiplier", self.threshold_init_multiplier)?;
        if self.phase_plan != full_phase_plan() {
            return Err(S2SchemaError::InvalidPhasePlan(
                "TrainConfigS2Full must use D1 PhaseA..PhaseD boundaries",
            ));
        }
        Ok(())
    }
}

impl Default for TrainConfigS2Full {
    fn default() -> Self {
        Self::pinned()
    }
}

/// Phase-A-only S2 config used by the ablation comparator.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrainConfigS2PhaseAOnly {
    /// Number of Phase A optimizer steps.
    pub optimizer_steps: u64,
    /// Number of sampled sequences per optimizer step.
    pub batch_size: usize,
    /// Number of bytes per sampled sequence.
    pub sequence_length: usize,
    /// Progress evaluation cadence.
    pub eval_every_steps: u64,
    /// Progress evaluation subset size in sequences.
    pub eval_subset_size: u64,
    /// AdamW optimizer configuration.
    pub optimizer: AdamWConfig,
    /// Phase A-only plan.
    pub phase_plan: Vec<TrainConfigS2Phase>,
    /// Deterministic RNG kind.
    pub rng_kind: RngKind,
    /// Required deterministic CPU device profile.
    pub device_profile: S1CpuDeterministic,
}

impl TrainConfigS2PhaseAOnly {
    /// Return the RFC-pinned Phase A-only config.
    #[must_use]
    pub fn pinned() -> Self {
        Self {
            optimizer_steps: S2_TEACHER_FREEZE_STEP,
            batch_size: S1_BATCH_SIZE,
            sequence_length: S1_SEQUENCE_LENGTH,
            eval_every_steps: S1_EVAL_EVERY_STEPS,
            eval_subset_size: S1_EVAL_SUBSET_SIZE,
            optimizer: AdamWConfig::pinned(),
            phase_plan: phase_a_plan(),
            rng_kind: RngKind::Pcg64Mcg,
            device_profile: S1CpuDeterministic::canonical(),
        }
    }
}

impl Default for TrainConfigS2PhaseAOnly {
    fn default() -> Self {
        Self::pinned()
    }
}

/// `s2_phase_log.v1` scheduler event embedded in per-step JSONL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PhaseEvent {
    /// First step of a new phase.
    PhaseTransition {
        /// Previous phase.
        from: PhaseKindS2,
        /// New phase.
        to: PhaseKindS2,
    },
    /// Teacher freeze audit marker.
    TeacherFreeze {
        /// Frozen teacher checkpoint hash.
        teacher_checkpoint_sha: Hash256,
    },
}

/// One canonical JSONL row in `s2_phase_log.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseEntry {
    /// 1-indexed optimizer step.
    pub step: GlobalStep,
    /// Phase active for this step.
    pub phase: PhaseKindS2,
    /// Effective QAT hardness after build-kind override.
    pub hardness: HardnessTriple,
    /// Router mode for this step.
    pub router_mode: RouterTrainMode,
    /// Phase-effective loss lambdas.
    pub lambda_effective: PhaseEffectiveLambda,
    /// Whether the teacher is frozen for this step.
    pub teacher_frozen: bool,
    /// Total train loss diagnostic.
    pub train_loss: f32,
    /// Global gradient norm diagnostic.
    pub grad_norm: f32,
    /// Raw distillation loss diagnostic, when applicable.
    pub distill_loss: Option<DistillLossNats>,
    /// Scheduler events emitted at this step.
    pub events: Vec<PhaseEvent>,
}

impl PhaseEntry {
    /// Validate PL-7/PL-8 and per-entry scalar shape.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.step == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PhaseEntry step must be 1-indexed",
            ));
        }
        validate_finite("train_loss", self.train_loss)?;
        validate_finite_nonnegative("grad_norm", self.grad_norm)?;
        if let Some(loss) = self.distill_loss {
            validate_finite_nonnegative("distill_loss", loss)?;
        }
        self.lambda_effective.validate()?;
        Ok(())
    }
}

/// Header for `s2_phase_log.v1`; entries are stored as ordered JSONL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseLog {
    /// Schema id. Expected value: `s2_phase_log.v1`.
    pub schema: String,
    /// Seed id.
    pub seed: u64,
    /// Runtime build kind.
    pub build_kind: S2BuildKind,
    /// Train configuration hash.
    pub train_config_hash: Hash256,
    /// Full S2 D1 phase boundaries.
    pub full_s2_phase_boundaries: Vec<u64>,
    /// Checkpoint steps executed by the run.
    pub executed_checkpoint_steps: Vec<u64>,
    /// D2 hardness ramp id.
    pub hardness_ramp_id: String,
    /// Phase A teacher freeze step.
    pub teacher_freeze_step: u64,
    /// Number of optimizer entries expected in JSONL.
    pub optimizer_steps: u64,
    /// Header+entry self hash.
    pub phase_log_self_hash: Hash256,
}

impl PhaseLog {
    /// Construct and hash a phase log header for the supplied entries.
    pub fn new(
        seed: u64,
        build_kind: S2BuildKind,
        train_config_hash: Hash256,
        executed_checkpoint_steps: Vec<u64>,
        entries: &[PhaseEntry],
    ) -> Result<Self, S1SchemaError> {
        let optimizer_steps = expected_phase_log_steps(build_kind);
        Self {
            schema: PHASE_LOG_SCHEMA.to_owned(),
            seed,
            build_kind,
            train_config_hash,
            full_s2_phase_boundaries: vec![4_000, 5_000, 8_000, 10_000],
            executed_checkpoint_steps,
            hardness_ramp_id: "PhaseCRampD2+PhaseDRampD2".to_owned(),
            teacher_freeze_step: S2_TEACHER_FREEZE_STEP,
            optimizer_steps,
            phase_log_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash(entries)
    }

    /// Validate PL-0..PL-8 against ordered entries.
    pub fn validate(&self, entries: &[PhaseEntry]) -> Result<(), S2SchemaError> {
        if self.schema != PHASE_LOG_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PhaseLog schema must be s2_phase_log.v1",
            ));
        }
        if self.full_s2_phase_boundaries != [4_000, 5_000, 8_000, 10_000] {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PhaseLog full_s2_phase_boundaries must match D1",
            ));
        }
        if self.hardness_ramp_id != "PhaseCRampD2+PhaseDRampD2" {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PhaseLog hardness_ramp_id must match D2",
            ));
        }
        if self.teacher_freeze_step != S2_TEACHER_FREEZE_STEP {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PhaseLog teacher_freeze_step must be 4000",
            ));
        }
        if self.optimizer_steps != expected_phase_log_steps(self.build_kind)
            || entries.len() as u64 != self.optimizer_steps
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PL-1 requires entry count to equal optimizer_steps",
            ));
        }
        let mut transition_event_count = 0_u32;
        let mut teacher_freeze_event_count = 0_u32;
        for (index, entry) in entries.iter().enumerate() {
            entry.validate()?;
            let expected_step = index as u64 + 1;
            if entry.step != expected_step {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "PhaseEntry steps must be contiguous and 1-indexed",
                ));
            }
            validate_phase_entry_for_build(entry, self.build_kind)?;
            for event in &entry.events {
                match event {
                    PhaseEvent::PhaseTransition { from, to } => {
                        transition_event_count += 1;
                        if !matches_phase_transition(entry.step, *from, *to) {
                            return Err(S2SchemaError::InvalidVerifierReport(
                                "PL-6 transition events must occur at D1 boundaries",
                            ));
                        }
                    }
                    PhaseEvent::TeacherFreeze { .. } => {
                        teacher_freeze_event_count += 1;
                        if entry.step != S2_TEACHER_FREEZE_STEP + 1 {
                            return Err(S2SchemaError::InvalidVerifierReport(
                                "teacher freeze event must occur at step 4001",
                            ));
                        }
                    }
                }
            }
        }
        let expected_transition_count = if self.build_kind == S2BuildKind::s2_ablation {
            0
        } else {
            3
        };
        let expected_teacher_freeze_count = if self.build_kind == S2BuildKind::s2_ablation {
            0
        } else {
            1
        };
        if transition_event_count != expected_transition_count {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PL-2 requires the expected PhaseTransition event count",
            ));
        }
        if teacher_freeze_event_count != expected_teacher_freeze_count {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PL-3 requires the expected TeacherFreeze event count",
            ));
        }
        Ok(())
    }

    /// Canonical header JSON bytes omitting `phase_log_self_hash`.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "phase_log_self_hash")
    }

    /// Ordered canonical JSONL bytes for entries.
    pub fn canonical_jsonl_bytes(entries: &[PhaseEntry]) -> Result<Vec<u8>, S1SchemaError> {
        let mut bytes = Vec::new();
        for entry in entries {
            bytes.extend_from_slice(&S1CanonicalJson::to_vec(entry)?);
            bytes.push(b'\n');
        }
        Ok(bytes)
    }

    /// Compute PL-0 header+entries self-hash.
    pub fn computed_self_hash(&self, entries: &[PhaseEntry]) -> Result<Hash256, S1SchemaError> {
        let mut bytes = self.canonical_json_bytes()?;
        bytes.push(0);
        bytes.extend_from_slice(PHASE_LOG_HASH_DOMAIN_SEPARATOR);
        bytes.extend_from_slice(&Self::canonical_jsonl_bytes(entries)?);
        Ok(gbf_foundation::sha256(bytes))
    }

    /// Return a copy with `phase_log_self_hash` recomputed.
    pub fn with_computed_self_hash(
        mut self,
        entries: &[PhaseEntry],
    ) -> Result<Self, S1SchemaError> {
        self.validate(entries)
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.phase_log_self_hash = self.computed_self_hash(entries)?;
        Ok(self)
    }
}

/// Threshold statistics embedded in `s2_score.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThresholdStatsSummary {
    /// Number of matrices summarized.
    pub matrices: u32,
    /// Minimum threshold.
    pub threshold_min: f32,
    /// Maximum threshold.
    pub threshold_max: f32,
    /// Mean threshold.
    pub threshold_mean: f32,
    /// Number of threshold values.
    pub threshold_count: u32,
}

impl ThresholdStatsSummary {
    /// Validate threshold aggregate invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.matrices == 0 || self.threshold_count == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "threshold stats require non-zero matrices and threshold_count",
            ));
        }
        validate_finite("threshold_min", self.threshold_min)?;
        validate_finite("threshold_max", self.threshold_max)?;
        validate_finite("threshold_mean", self.threshold_mean)?;
        if self.threshold_min > self.threshold_mean || self.threshold_mean > self.threshold_max {
            return Err(S2SchemaError::InvalidVerifierReport(
                "threshold stats require min <= mean <= max",
            ));
        }
        Ok(())
    }
}

/// QAT scale statistics embedded in `s2_score.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaleStatsSummary {
    /// Number of matrices summarized.
    pub matrices: u32,
    /// Number of scale values.
    pub scale_count: u32,
    /// Minimum scale.
    pub scale_min: f32,
    /// Maximum scale.
    pub scale_max: f32,
    /// Mean scale.
    pub scale_mean_f32: f32,
}

impl ScaleStatsSummary {
    /// Validate scale aggregate invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.matrices == 0 || self.scale_count == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "scale stats require non-zero matrices and scale_count",
            ));
        }
        validate_finite("scale_min", self.scale_min)?;
        validate_finite("scale_max", self.scale_max)?;
        validate_finite("scale_mean_f32", self.scale_mean_f32)?;
        if self.scale_min > self.scale_mean_f32 || self.scale_mean_f32 > self.scale_max {
            return Err(S2SchemaError::InvalidVerifierReport(
                "scale stats require min <= mean <= max",
            ));
        }
        Ok(())
    }
}

/// `s2_score.v1` score artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2ScoreReport {
    /// Schema id. Expected value: `s2_score.v1`.
    pub schema: String,
    /// Seed id.
    pub seed: u64,
    /// Runtime build kind.
    pub build_kind: S2BuildKind,
    /// Scored checkpoint hash.
    pub checkpoint_sha: Hash256,
    /// Validation corpus hash.
    pub corpus_val_sha: Hash256,
    /// Chunk size used for scoring.
    pub chunk_size: u32,
    /// Token count scored.
    pub token_count: u64,
    /// Sum of log2 losses.
    pub log2_sum: f64,
    /// Bits per byte.
    pub bpc: f64,
    /// Ternary threshold statistics.
    pub threshold_stats: Option<ThresholdStatsSummary>,
    /// Ternary/Q8.8 scale statistics.
    pub scale_stats: Option<ScaleStatsSummary>,
    /// Score artifact self-hash.
    pub score_self_hash: Hash256,
}

impl S2ScoreReport {
    /// Construct and hash an `s2_score.v1` report.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        build_kind: S2BuildKind,
        checkpoint_sha: Hash256,
        corpus_val_sha: Hash256,
        token_count: u64,
        log2_sum: f64,
        threshold_stats: Option<ThresholdStatsSummary>,
        scale_stats: Option<ScaleStatsSummary>,
    ) -> Result<Self, S1SchemaError> {
        let bpc = log2_sum / token_count as f64;
        Self {
            schema: S2_SCORE_SCHEMA.to_owned(),
            seed,
            build_kind,
            checkpoint_sha,
            corpus_val_sha,
            chunk_size: 128,
            token_count,
            log2_sum,
            bpc,
            threshold_stats,
            scale_stats,
            score_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate score invariants and stats nullability.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != S2_SCORE_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "S2ScoreReport schema must be s2_score.v1",
            ));
        }
        if self.chunk_size != 128 || self.token_count == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s2_score.v1 requires chunk_size=128 and token_count>0",
            ));
        }
        validate_finite_nonnegative_f64("log2_sum", self.log2_sum)?;
        validate_finite_nonnegative_f64("bpc", self.bpc)?;
        if (self.log2_sum / self.token_count as f64 - self.bpc).abs() > 1.0e-12 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s2_score.v1 bpc must equal log2_sum/token_count",
            ));
        }
        let needs_qat_stats = matches!(
            self.build_kind,
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
        );
        if needs_qat_stats != self.threshold_stats.is_some()
            || needs_qat_stats != self.scale_stats.is_some()
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s2_score.v1 QAT stats nullability must match build kind",
            ));
        }
        if let Some(stats) = &self.threshold_stats {
            stats.validate()?;
        }
        if let Some(stats) = &self.scale_stats {
            stats.validate()?;
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting `score_self_hash`.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "score_self_hash")
    }

    /// Compute the report self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact("S2ScoreReport", S2_SCORE_SCHEMA, self, "score_self_hash")
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.score_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// One distillation diagnostic point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistillEvalPoint {
    /// Evaluation step.
    pub eval_step: GlobalStep,
    /// Raw distillation loss at this point.
    pub distill_loss: Option<DistillLossNats>,
}

impl DistillEvalPoint {
    /// Validate scalar shape.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.eval_step == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "DistillEvalPoint eval_step must be 1-indexed",
            ));
        }
        if let Some(loss) = self.distill_loss {
            validate_finite_nonnegative("distill_loss", loss)?;
        }
        Ok(())
    }
}

/// Per-eval-point raw and weighted S2 loss diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossTermEvalPoint {
    /// Evaluation step.
    pub eval_step: GlobalStep,
    /// Phase-effective lambdas at this point.
    pub lambda_effective: PhaseEffectiveLambda,
    /// Raw loss values by term; `null` means structurally inert.
    pub raw_losses: BTreeMap<String, Option<f32>>,
    /// Weighted loss values by term; `null` means structurally inert.
    pub weighted_losses: BTreeMap<String, Option<f32>>,
}

impl LossTermEvalPoint {
    /// Validate loss maps and lambda shape.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.eval_step == 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "LossTermEvalPoint eval_step must be 1-indexed",
            ));
        }
        self.lambda_effective.validate()?;
        if self.raw_losses.keys().collect::<Vec<_>>()
            != self.weighted_losses.keys().collect::<Vec<_>>()
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "raw_losses and weighted_losses must use identical term keys",
            ));
        }
        for value in self
            .raw_losses
            .values()
            .chain(self.weighted_losses.values())
        {
            if let Some(value) = *value {
                validate_finite_nonnegative("loss_term_value", value)?;
            }
        }
        for (term, raw_loss) in &self.raw_losses {
            let weighted_loss =
                self.weighted_losses
                    .get(term)
                    .ok_or(S2SchemaError::InvalidVerifierReport(
                        "raw_losses and weighted_losses must use identical term keys",
                    ))?;
            if raw_loss.is_some() != weighted_loss.is_some() {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "LossTermEvalPoint raw and weighted nullability must match",
                ));
            }
        }
        Ok(())
    }
}

/// `s2_distillation_log.v1` artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DistillationLog {
    /// Schema id. Expected value: `s2_distillation_log.v1`.
    pub schema: String,
    /// Seed id.
    pub seed: u64,
    /// Runtime build kind.
    pub build_kind: S2BuildKind,
    /// Frozen teacher checkpoint hash.
    pub teacher_checkpoint_sha: Hash256,
    /// Teacher weight fingerprint.
    pub teacher_weight_fingerprint: Hash256,
    /// Teacher storage fingerprint.
    pub teacher_storage_fingerprint: Hash256,
    /// Teacher freeze step.
    pub teacher_freeze_step: GlobalStep,
    /// Distillation temperature.
    pub distill_temperature: DistillTemperature,
    /// Configured default distillation lambda.
    pub lambda_distill_default: f32,
    /// Raw distillation losses at eval points.
    pub distill_loss_per_eval_point: Vec<DistillEvalPoint>,
    /// Authoritative phase log self-hash.
    pub phase_log_self_hash: Hash256,
    /// Raw/weighted term diagnostics at eval points.
    pub loss_terms_per_eval_point: Vec<LossTermEvalPoint>,
    /// Distillation artifact self-hash.
    pub distill_log_self_hash: Hash256,
}

impl DistillationLog {
    /// Construct and hash an `s2_distillation_log.v1` report.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        build_kind: S2BuildKind,
        teacher_checkpoint_sha: Hash256,
        teacher_weight_fingerprint: Hash256,
        teacher_storage_fingerprint: Hash256,
        distill_temperature: DistillTemperature,
        lambda_distill_default: f32,
        distill_loss_per_eval_point: Vec<DistillEvalPoint>,
        phase_log_self_hash: Hash256,
        loss_terms_per_eval_point: Vec<LossTermEvalPoint>,
    ) -> Result<Self, S1SchemaError> {
        Self {
            schema: DISTILLATION_LOG_SCHEMA.to_owned(),
            seed,
            build_kind,
            teacher_checkpoint_sha,
            teacher_weight_fingerprint,
            teacher_storage_fingerprint,
            teacher_freeze_step: S2_TEACHER_FREEZE_STEP,
            distill_temperature,
            lambda_distill_default,
            distill_loss_per_eval_point,
            phase_log_self_hash,
            loss_terms_per_eval_point,
            distill_log_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate DL-1..DL-3 shape owned by this schema bead.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != DISTILLATION_LOG_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "DistillationLog schema must be s2_distillation_log.v1",
            ));
        }
        if self.teacher_freeze_step != S2_TEACHER_FREEZE_STEP {
            return Err(S2SchemaError::InvalidVerifierReport(
                "DistillationLog teacher_freeze_step must be 4000",
            ));
        }
        validate_finite_positive("distill_temperature", self.distill_temperature)?;
        validate_finite_nonnegative("lambda_distill_default", self.lambda_distill_default)?;
        let expected_lambda = lambda_distill_default_for_build_kind(1.0, self.build_kind);
        if (self.lambda_distill_default - expected_lambda).abs() > f32::EPSILON {
            return Err(S2SchemaError::InvalidVerifierReport(
                "lambda_distill_default must match build kind",
            ));
        }
        if self.distill_loss_per_eval_point.is_empty()
            || self.loss_terms_per_eval_point.is_empty()
            || self.distill_loss_per_eval_point.len() != self.loss_terms_per_eval_point.len()
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "DistillationLog eval point vectors must be non-empty and aligned",
            ));
        }
        for (distill, terms) in self
            .distill_loss_per_eval_point
            .iter()
            .zip(&self.loss_terms_per_eval_point)
        {
            distill.validate()?;
            terms.validate()?;
            if distill.eval_step != terms.eval_step {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "DistillationLog eval point steps must align",
                ));
            }
            let pre_phase_c = distill.eval_step <= S2_PHASE_B_END_STEP;
            if pre_phase_c && distill.distill_loss.is_some() {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "pre-Phase-C distill_loss must be null",
                ));
            }
            if !pre_phase_c
                && self.build_kind != S2BuildKind::s2_ablation
                && distill.distill_loss.is_none()
            {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "Phase C/D distill_loss must be finite for full builds",
                ));
            }
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting `distill_log_self_hash`.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "distill_log_self_hash")
    }

    /// Compute the report self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        let mut bytes = b"gbf-experiments:DistillationLog:s2_distillation_log.v1:1\0".to_vec();
        bytes.extend_from_slice(&self.canonical_json_bytes()?);
        Ok(gbf_foundation::sha256(bytes))
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.distill_log_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// Diagnostic subcheck nested under an S2 loss-gradient fixture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSubcheckResult {
    /// Diagnostic subcheck name.
    pub name: String,
    /// Lambda value used by the diagnostic.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub lambda_value: f32,
    /// Whether the raw diagnostic was computed.
    pub raw_loss_computed: bool,
    /// Whether the raw diagnostic value was finite.
    pub raw_loss_finite: bool,
    /// Weighted loss value when applicable.
    #[serde(default, deserialize_with = "optional_finite_nonnegative_f32")]
    pub weighted_loss_value: Option<f32>,
    /// Subcheck verdict.
    pub passed: bool,
}

impl DiagnosticSubcheckResult {
    /// Validate scalar invariants for this diagnostic subcheck.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        validate_finite_nonnegative("lambda_value", self.lambda_value)?;
        if let Some(value) = self.weighted_loss_value {
            validate_finite_nonnegative("weighted_loss_value", value)?;
        }
        if self.passed && (!self.raw_loss_computed || !self.raw_loss_finite) {
            return Err(S2SchemaError::InvalidVerifierReport(
                "passed diagnostic subchecks must compute a finite raw loss",
            ));
        }
        Ok(())
    }
}

/// Per-loss fixture result for `s2_loss_grad_flow.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureResult {
    /// Sub-hypothesis id, H5.1 through H5.5.
    pub sub_hypothesis: String,
    /// Loss-term name.
    pub loss_term: String,
    /// Grad norms that must receive gradient.
    #[serde(deserialize_with = "finite_nonnegative_f32_map")]
    pub in_scope_grad_norms: BTreeMap<String, f32>,
    /// Grad norms that must be zero within epsilon.
    #[serde(deserialize_with = "finite_nonnegative_f32_map")]
    pub stop_gradient_grad_norms: BTreeMap<String, f32>,
    /// Whether the fixture used a non-default/non-1.0 scalar.
    pub non_default_value_used: bool,
    /// Whether numerical stability checks passed.
    pub numerical_stability_passed: bool,
    /// Nested diagnostic subchecks.
    pub diagnostic_subchecks: Vec<DiagnosticSubcheckResult>,
    /// Detached-gradient absent-entry markers, used by H5.5.
    pub detached_grad_absence: BTreeMap<String, bool>,
    /// Fixture verdict.
    pub sub_passed: bool,
}

impl FixtureResult {
    /// Validate LGF fixture invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if !matches!(
            self.sub_hypothesis.as_str(),
            "H5.1" | "H5.2" | "H5.3" | "H5.4" | "H5.5"
        ) {
            return Err(S2SchemaError::InvalidVerifierReport(
                "loss-grad-flow sub_hypothesis must be H5.1 through H5.5",
            ));
        }
        if !matches!(
            self.loss_term.as_str(),
            "lambda_zrouter" | "lambda_balance" | "lambda_range" | "lambda_zero" | "lambda_distill"
        ) {
            return Err(S2SchemaError::InvalidVerifierReport(
                "loss-grad-flow loss_term must be an S2 loss lambda",
            ));
        }
        if !self.non_default_value_used {
            return Err(S2SchemaError::InvalidVerifierReport(
                "LGF-2 requires non_default_value_used=true",
            ));
        }
        for (name, value) in &self.in_scope_grad_norms {
            validate_finite_nonnegative_map_value("in_scope_grad_norms", name, *value)?;
        }
        for (name, value) in &self.stop_gradient_grad_norms {
            validate_finite_nonnegative_map_value("stop_gradient_grad_norms", name, *value)?;
            if *value > STOP_GRAD_EPS {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "stop_gradient_grad_norms must be <= 1e-6",
                ));
            }
        }
        for (name, detached_absent) in &self.detached_grad_absence {
            if *detached_absent
                && self
                    .stop_gradient_grad_norms
                    .get(name)
                    .is_some_and(|value| *value != 0.0)
            {
                return Err(S2SchemaError::InvalidVerifierReport(
                    "detached_grad_absence=true requires an absent or exactly-zero stop-gradient entry",
                ));
            }
        }
        for subcheck in &self.diagnostic_subchecks {
            subcheck.validate()?;
        }
        let h5_5_teacher_exact_zero =
            self.stop_gradient_grad_norms.get("teacher_logits") == Some(&0.0);
        let h5_5_teacher_detached_absent = self
            .detached_grad_absence
            .get("teacher_logits")
            .copied()
            .unwrap_or(false)
            && !self.stop_gradient_grad_norms.contains_key("teacher_logits");
        if self.sub_hypothesis == "H5.5"
            && !(h5_5_teacher_exact_zero || h5_5_teacher_detached_absent)
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "LGF-4 requires H5.5 teacher_logits stop-gradient to be exactly 0.0 or detached-absent",
            ));
        }
        if self.sub_hypothesis == "H5.4"
            && !self.diagnostic_subchecks.iter().any(|subcheck| {
                subcheck.name == "lambda_zero_raw_honesty_at_zero_weight"
                    && subcheck.lambda_value == 0.0
            })
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "LGF-5 requires H5.4 lambda_zero_raw_honesty_at_zero_weight with lambda_value=0.0",
            ));
        }
        let expected_passed = self.numerical_stability_passed
            && self
                .diagnostic_subchecks
                .iter()
                .all(|subcheck| subcheck.passed)
            && self
                .in_scope_grad_norms
                .values()
                .all(|value| value.is_finite() && *value > 0.0)
            && self
                .stop_gradient_grad_norms
                .values()
                .all(|value| value.is_finite() && *value <= STOP_GRAD_EPS);
        if self.sub_passed != expected_passed {
            return Err(S2SchemaError::InvalidVerifierReport(
                "FixtureResult sub_passed must match LGF invariant conjunction",
            ));
        }
        Ok(())
    }
}

/// `s2_loss_grad_flow.v1` report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LossGradFlowReport {
    /// Schema id.
    pub schema: String,
    /// Five H5 fixture results.
    pub fixtures: Vec<FixtureResult>,
    /// Aggregate verdict.
    pub overall_passed: bool,
    /// Self-hash with this field omitted.
    pub loss_grad_flow_self_hash: Hash256,
}

impl LossGradFlowReport {
    /// Construct and hash a loss-gradient-flow report.
    pub fn new(fixtures: Vec<FixtureResult>) -> Result<Self, S1SchemaError> {
        let overall_passed = fixtures.iter().all(|fixture| fixture.sub_passed);
        Self {
            schema: LOSS_GRAD_FLOW_SCHEMA.to_owned(),
            fixtures,
            overall_passed,
            loss_grad_flow_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate LGF report invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != LOSS_GRAD_FLOW_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "LossGradFlowReport schema must be s2_loss_grad_flow.v1",
            ));
        }
        if self.fixtures.len() != 5 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s2_loss_grad_flow.v1 requires exactly five fixtures",
            ));
        }
        for fixture in &self.fixtures {
            fixture.validate()?;
        }
        let observed = self
            .fixtures
            .iter()
            .map(|fixture| fixture.sub_hypothesis.as_str())
            .collect::<Vec<_>>();
        if observed != ["H5.1", "H5.2", "H5.3", "H5.4", "H5.5"] {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s2_loss_grad_flow.v1 fixtures must be ordered H5.1..H5.5",
            ));
        }
        if self.overall_passed != self.fixtures.iter().all(|fixture| fixture.sub_passed) {
            return Err(S2SchemaError::InvalidVerifierReport(
                "overall_passed must equal AND of fixture sub_passed values",
            ));
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting the report self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "loss_grad_flow_self_hash")
    }

    /// Compute the report self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact(
            "LossGradFlowReport",
            LOSS_GRAD_FLOW_SCHEMA,
            self,
            "loss_grad_flow_self_hash",
        )
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.loss_grad_flow_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// `s2_linearstate_grad_smoke.v1` report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinearStateSmokeReport {
    /// Schema id.
    pub schema: String,
    /// Fixture id.
    pub fixture_id: String,
    /// Sequence length.
    #[serde(rename = "fixture_seq_len")]
    pub seq_len: u64,
    /// Hidden dimension.
    #[serde(rename = "fixture_hidden_dim")]
    pub hidden_dim: u64,
    /// Batch size.
    #[serde(rename = "fixture_batch")]
    pub batch: u64,
    /// Whether forward outputs were finite.
    pub forward_finite: bool,
    /// Parameter gradient norms.
    #[serde(deserialize_with = "finite_nonnegative_f32_map")]
    pub param_grad_norms: BTreeMap<String, f32>,
    /// Parameters intentionally inactive in this fixture; LS-2 does not require non-zero gradients for them.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub inactive_parameters: BTreeSet<String>,
    /// Input gradient norm.
    #[serde(deserialize_with = "finite_nonnegative_f32")]
    pub input_grad_norm: f32,
    /// Deterministic byte equality verdict.
    pub determinism_byte_equal: bool,
    /// Aggregate smoke verdict.
    pub smoke_passed: bool,
    /// Self-hash with this field omitted.
    pub smoke_self_hash: Hash256,
}

impl LinearStateSmokeReport {
    /// Construct and hash a LinearState smoke report.
    pub fn new(
        param_grad_norms: BTreeMap<String, f32>,
        input_grad_norm: f32,
    ) -> Result<Self, S1SchemaError> {
        Self {
            schema: LINEARSTATE_SMOKE_SCHEMA.to_owned(),
            fixture_id: "FIXTURE_V1".to_owned(),
            seq_len: 8,
            hidden_dim: 4,
            batch: 1,
            forward_finite: true,
            param_grad_norms,
            inactive_parameters: BTreeSet::new(),
            input_grad_norm,
            determinism_byte_equal: true,
            smoke_passed: true,
            smoke_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate LS report invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != LINEARSTATE_SMOKE_SCHEMA
            || self.fixture_id != "FIXTURE_V1"
            || self.seq_len != 8
            || self.hidden_dim != 4
            || self.batch != 1
        {
            let error = S2SchemaError::InvalidVerifierReport(
                "LinearStateSmokeReport must use FIXTURE_V1 dimensions",
            );
            log_linearstate_invariant_failed(
                "LS-1",
                None,
                serde_json::json!({
                    "schema": &self.schema,
                    "fixture_id": &self.fixture_id,
                    "fixture_seq_len": self.seq_len,
                    "fixture_hidden_dim": self.hidden_dim,
                    "fixture_batch": self.batch,
                }),
            );
            return Err(error);
        }
        for (name, value) in &self.param_grad_norms {
            if let Err(error) =
                validate_finite_nonnegative_map_value("param_grad_norms", name, *value)
            {
                log_linearstate_invariant_failed(
                    "LS-2",
                    Some(name),
                    serde_json::json!({ "observed_grad_norm": value }),
                );
                return Err(error);
            }
        }
        for name in &self.inactive_parameters {
            if name.is_empty() {
                let error = S2SchemaError::InvalidVerifierReport(
                    "inactive LinearState parameter names must be non-empty",
                );
                log_linearstate_invariant_failed(
                    "LS-2",
                    Some(name),
                    serde_json::json!({ "inactive_parameter": name }),
                );
                return Err(error);
            }
        }
        if let Err(error) = validate_finite_nonnegative("input_grad_norm", self.input_grad_norm) {
            log_linearstate_invariant_failed(
                "LS-3",
                None,
                serde_json::json!({ "input_grad_norm": self.input_grad_norm }),
            );
            return Err(error);
        }
        let failed_active_parameter = self.param_grad_norms.iter().find(|(name, value)| {
            !self.inactive_parameters.contains(*name) && (!value.is_finite() || **value <= 0.0)
        });
        let expected_passed = self.forward_finite
            && self.determinism_byte_equal
            && self.input_grad_norm > 0.0
            && self.param_grad_norms.iter().all(|(name, value)| {
                self.inactive_parameters.contains(name) || (value.is_finite() && *value > 0.0)
            });
        if self.smoke_passed != expected_passed {
            if !self.forward_finite {
                log_linearstate_invariant_failed(
                    "LS-1",
                    None,
                    serde_json::json!({ "forward_finite": self.forward_finite }),
                );
            } else if let Some((name, value)) = failed_active_parameter {
                log_linearstate_invariant_failed(
                    "LS-2",
                    Some(name),
                    serde_json::json!({
                        "observed_grad_norm": value,
                        "inactive": self.inactive_parameters.contains(name),
                    }),
                );
            } else if self.input_grad_norm <= 0.0 {
                log_linearstate_invariant_failed(
                    "LS-3",
                    None,
                    serde_json::json!({ "input_grad_norm": self.input_grad_norm }),
                );
            } else if !self.determinism_byte_equal {
                log_linearstate_invariant_failed(
                    "LS-4",
                    None,
                    serde_json::json!({ "determinism_byte_equal": self.determinism_byte_equal }),
                );
            } else {
                log_linearstate_invariant_failed(
                    "LS-5",
                    None,
                    serde_json::json!({
                        "smoke_passed": self.smoke_passed,
                        "expected_passed": expected_passed,
                    }),
                );
            }
            return Err(S2SchemaError::InvalidVerifierReport(
                "smoke_passed must match LS invariant conjunction",
            ));
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting the report self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "smoke_self_hash")
    }

    /// Compute the report self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact(
            "LinearStateSmokeReport",
            LINEARSTATE_SMOKE_SCHEMA,
            self,
            "smoke_self_hash",
        )
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.smoke_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// `s2_phase_transition_integration.v1` report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseTransitionIntegReport {
    /// Schema id.
    pub schema: String,
    /// Fixture id.
    pub fixture_id: String,
    /// Fixture phase boundaries for half-open intervals.
    pub fixture_phase_boundaries: Vec<u32>,
    /// Number of transition events observed.
    pub transition_event_count: u32,
    /// Number of teacher-freeze events observed.
    pub teacher_freeze_event_count: u32,
    /// Hardness observed at fixture boundaries.
    pub hardness_at_boundary: BTreeMap<String, HardnessTriple>,
    /// Whether skip-phase rejection was tested.
    pub skip_phase_test_passed: bool,
    /// Whether overlap rejection was raised.
    pub overlap_phase_error_raised: bool,
    /// Whether empty-phase rejection was raised.
    pub empty_phase_error_raised: bool,
    /// Aggregate integration verdict.
    pub integ_passed: bool,
    /// Self-hash with this field omitted.
    pub integ_self_hash: Hash256,
}

impl PhaseTransitionIntegReport {
    /// Construct and hash a phase-transition integration report.
    pub fn new(
        transition_event_count: u32,
        teacher_freeze_event_count: u32,
        hardness_at_boundary: BTreeMap<String, HardnessTriple>,
        skip_phase_test_passed: bool,
        overlap_phase_error_raised: bool,
        empty_phase_error_raised: bool,
    ) -> Result<Self, S1SchemaError> {
        let integ_passed = transition_event_count == 4
            && teacher_freeze_event_count == 1
            && skip_phase_test_passed
            && overlap_phase_error_raised
            && empty_phase_error_raised;
        Self {
            schema: PHASE_TRANSITION_INTEG_SCHEMA.to_owned(),
            fixture_id: "tiny_model_T10.1".to_owned(),
            fixture_phase_boundaries: canonical_fixture_phase_boundaries(),
            transition_event_count,
            teacher_freeze_event_count,
            hardness_at_boundary,
            skip_phase_test_passed,
            overlap_phase_error_raised,
            empty_phase_error_raised,
            integ_passed,
            integ_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate PT report invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != PHASE_TRANSITION_INTEG_SCHEMA || self.fixture_id != "tiny_model_T10.1" {
            let error = S2SchemaError::InvalidVerifierReport(
                "PhaseTransitionIntegReport must use tiny_model_T10.1 schema",
            );
            log_phase_transition_invariant_failed(
                "PT-1",
                None,
                serde_json::json!({
                    "schema": &self.schema,
                    "fixture_id": &self.fixture_id,
                }),
            );
            return Err(error);
        }
        let pt1 = self.transition_event_count == 4
            && self.fixture_phase_boundaries == canonical_fixture_phase_boundaries();
        let pt2 = self.teacher_freeze_event_count == 1;
        let expected_hardness = phase_transition_expected_hardness_at_boundary();
        let expected_boundaries = expected_hardness.keys().cloned().collect::<Vec<_>>();
        let observed_hardness = parsed_hardness_at_boundary(&self.hardness_at_boundary);
        let pt3 = observed_hardness
            .as_ref()
            .map(|observed| observed.keys().copied().collect::<Vec<_>>() == [10, 20, 30, 40])
            .unwrap_or(false)
            && self.hardness_at_boundary == expected_hardness;
        let expected_passed = pt1
            && pt2
            && pt3
            && self.skip_phase_test_passed
            && self.overlap_phase_error_raised
            && self.empty_phase_error_raised;
        if self.integ_passed != expected_passed {
            if !pt1 {
                log_phase_transition_invariant_failed(
                    "PT-1",
                    None,
                    serde_json::json!({
                        "transition_event_count": self.transition_event_count,
                        "fixture_phase_boundaries": self.fixture_phase_boundaries,
                    }),
                );
            } else if !pt2 {
                log_phase_transition_invariant_failed(
                    "PT-2",
                    None,
                    serde_json::json!({
                        "teacher_freeze_event_count": self.teacher_freeze_event_count,
                    }),
                );
            } else if !pt3 {
                let boundary = first_mismatched_phase_boundary(&self.hardness_at_boundary);
                log_phase_transition_invariant_failed(
                    "PT-3",
                    boundary,
                    serde_json::json!({
                        "expected_boundaries": expected_boundaries,
                        "observed_boundaries": self.hardness_at_boundary.keys().collect::<Vec<_>>(),
                        "hardness_at_boundary": &self.hardness_at_boundary,
                    }),
                );
            } else if !self.skip_phase_test_passed {
                log_phase_transition_invariant_failed(
                    "PT-4",
                    None,
                    serde_json::json!({ "skip_phase_test_passed": self.skip_phase_test_passed }),
                );
            } else if !self.overlap_phase_error_raised {
                log_phase_transition_invariant_failed(
                    "PT-5",
                    None,
                    serde_json::json!({
                        "overlap_phase_error_raised": self.overlap_phase_error_raised,
                    }),
                );
            } else if !self.empty_phase_error_raised {
                log_phase_transition_invariant_failed(
                    "PT-6",
                    None,
                    serde_json::json!({
                        "empty_phase_error_raised": self.empty_phase_error_raised,
                    }),
                );
            } else {
                log_phase_transition_invariant_failed(
                    "PT-7",
                    None,
                    serde_json::json!({
                        "integ_passed": self.integ_passed,
                        "expected_passed": expected_passed,
                    }),
                );
            }
            return Err(S2SchemaError::InvalidVerifierReport(
                "integ_passed must match PT invariant conjunction",
            ));
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting the report self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "integ_self_hash")
    }

    /// Compute the report self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact(
            "PhaseTransitionIntegReport",
            PHASE_TRANSITION_INTEG_SCHEMA,
            self,
            "integ_self_hash",
        )
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.integ_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// Canonical D8 expected hardness rows keyed by fixture transition step.
#[must_use]
pub fn phase_transition_expected_hardness_at_boundary() -> BTreeMap<String, HardnessTriple> {
    phase_transition_expected_boundary_projections()
        .into_iter()
        .map(|projection| (projection.step.to_string(), projection.projected()))
        .collect()
}

/// Canonical D8 scheduler-control rows before projection.
#[must_use]
pub fn phase_transition_expected_boundary_projections() -> [PhaseBoundaryHardnessProjection; 4] {
    let off = HardnessTriple::all_off();
    let expert = HardnessTriple::new(
        QuantHardness::Hard,
        QuantHardness::Soft,
        QuantHardness::Soft,
    );
    let full = HardnessTriple::new(
        QuantHardness::Hard,
        QuantHardness::Hard,
        QuantHardness::Hard,
    );
    [
        PhaseBoundaryHardnessProjection::new(10, off, off),
        PhaseBoundaryHardnessProjection::new(20, off, expert),
        PhaseBoundaryHardnessProjection::new(30, expert, full),
        PhaseBoundaryHardnessProjection::new(40, full, full),
    ]
}

/// Write `s2_loss_grad_flow.v1` as canonical JSON.
pub fn write_loss_grad_flow_report(
    path: impl AsRef<Path>,
    report: &LossGradFlowReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "s2_loss_grad_flow_writer_open",
        path = %path.display(),
        "s2 loss-grad-flow writer open"
    );
    report.validate()?;
    for fixture in &report.fixtures {
        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "grad_flow_fixture_emit",
            sub_hypothesis = fixture.sub_hypothesis.as_str(),
            loss_term = fixture.loss_term.as_str(),
            sub_passed = fixture.sub_passed,
            non_default_value_used = fixture.non_default_value_used,
            diagnostic_subcheck_count = fixture.diagnostic_subchecks.len() as u32,
            "s2 grad-flow fixture emit"
        );
    }
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "loss_grad_flow_finalized",
        overall_passed = report.overall_passed,
        fixture_count = report.fixtures.len() as u32,
        loss_grad_flow_self_hash = %report.loss_grad_flow_self_hash,
        "s2 loss-grad-flow finalized"
    );
    Ok(())
}

/// Write `s2_linearstate_grad_smoke.v1` as canonical JSON.
pub fn write_linearstate_smoke_report(
    path: impl AsRef<Path>,
    report: &LinearStateSmokeReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "s2_linearstate_grad_smoke_writer_open",
        path = %path.display(),
        "s2 linearstate smoke writer open"
    );
    report.validate()?;
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "linearstate_smoke_finalized",
        smoke_passed = report.smoke_passed,
        smoke_self_hash = %report.smoke_self_hash,
        "s2 linearstate smoke finalized"
    );
    Ok(())
}

/// Write `s2_phase_transition_integration.v1` as canonical JSON.
pub fn write_phase_transition_integ_report(
    path: impl AsRef<Path>,
    report: &PhaseTransitionIntegReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "s2_phase_transition_integration_writer_open",
        path = %path.display(),
        "s2 phase-transition integration writer open"
    );
    report.validate()?;
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "phase_transition_integ_finalized",
        integ_passed = report.integ_passed,
        transition_event_count = report.transition_event_count,
        teacher_freeze_event_count = report.teacher_freeze_event_count,
        integ_self_hash = %report.integ_self_hash,
        "s2 phase-transition integration finalized"
    );
    Ok(())
}

/// Write `s2_phase_log.v1` header and ordered JSONL entries.
pub fn write_phase_log_artifacts(
    header_path: impl AsRef<Path>,
    jsonl_path: impl AsRef<Path>,
    header: &PhaseLog,
    entries: &[PhaseEntry],
) -> Result<(), S2ReportWriteError> {
    let header_path = header_path.as_ref();
    let jsonl_path = jsonl_path.as_ref();
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "phase_log_writer_open",
        build_kind = ?header.build_kind,
        seed = header.seed,
        header_path = %header_path.display(),
        jsonl_path = %jsonl_path.display(),
        "s2 phase-log writer open"
    );
    header.validate(entries)?;

    let mut header_writer = BufWriter::new(File::create(header_path)?);
    header_writer.write_all(&S1CanonicalJson::to_vec(header)?)?;
    header_writer.flush()?;
    header_writer.get_ref().sync_all()?;

    let flush_steps = phase_log_flush_steps(header.optimizer_steps);
    let mut jsonl_writer = BufWriter::new(File::create(jsonl_path)?);
    for (index, entry) in entries.iter().enumerate() {
        jsonl_writer.write_all(&S1CanonicalJson::to_vec(entry)?)?;
        jsonl_writer.write_all(b"\n")?;
        if flush_steps.contains(&entry.step) {
            jsonl_writer.flush()?;
            jsonl_writer.get_ref().sync_all()?;
            tracing::debug!(
                target: S2_LOG_TARGET,
                event_name = "phase_log_flush",
                entries_flushed = (index + 1) as u32,
                last_step = entry.step,
                fsync = true,
                "s2 phase-log flush"
            );
        }
    }
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "phase_log_finalized",
        entries = entries.len() as u32,
        transition_event_count = count_phase_transitions(entries),
        teacher_freeze_event_count = count_teacher_freezes(entries),
        phase_log_self_hash = %header.phase_log_self_hash,
        "s2 phase-log finalized"
    );
    Ok(())
}

fn phase_log_flush_steps(optimizer_steps: u64) -> Vec<u64> {
    let mut flush_steps = Vec::new();
    for step in [
        S2_TEACHER_FREEZE_STEP,
        S2_PHASE_B_END_STEP,
        S2_PHASE_C_END_STEP,
        optimizer_steps,
    ] {
        if step <= optimizer_steps && !flush_steps.contains(&step) {
            flush_steps.push(step);
        }
    }
    flush_steps
}

/// Write `s2_score.v1` as canonical JSON.
pub fn write_score_report(
    path: impl AsRef<Path>,
    report: &S2ScoreReport,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    report.validate()?;
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "score_emitter_finalized",
        build_kind = ?report.build_kind,
        seed = report.seed,
        bpc = report.bpc,
        score_self_hash = %report.score_self_hash,
        "s2 score emitter finalized"
    );
    Ok(())
}

/// Write `s2_distillation_log.v1` as canonical JSON.
pub fn write_distillation_log(
    path: impl AsRef<Path>,
    report: &DistillationLog,
) -> Result<(), S2ReportWriteError> {
    let path = path.as_ref();
    report.validate()?;
    fs::write(path, S1CanonicalJson::to_vec(report)?)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "distill_log_finalized",
        build_kind = ?report.build_kind,
        seed = report.seed,
        teacher_checkpoint_sha = %report.teacher_checkpoint_sha,
        distill_log_self_hash = %report.distill_log_self_hash,
        "s2 distillation log finalized"
    );
    Ok(())
}

/// Errors from S2 pre-train report writers.
#[derive(Debug)]
pub enum S2ReportWriteError {
    /// Report validation failed.
    Schema(S2SchemaError),
    /// Canonical JSON serialization failed.
    Canonical(S1SchemaError),
    /// Filesystem write failed.
    Io(std::io::Error),
}

impl fmt::Display for S2ReportWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema(error) => write!(f, "{error}"),
            Self::Canonical(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for S2ReportWriteError {}

impl From<S2SchemaError> for S2ReportWriteError {
    fn from(error: S2SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<S1SchemaError> for S2ReportWriteError {
    fn from(error: S1SchemaError) -> Self {
        Self::Canonical(error)
    }
}

impl From<std::io::Error> for S2ReportWriteError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Hash input for full S2 reproducibility.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct TrainConfigS2HashInput<'a> {
    config: &'a TrainConfigS2Full,
    build_kind: S2BuildKind,
    quant_hardness_override: QuantHardnessOverride,
    lambda_distill_effective_default: f32,
    environment_hash: S2EnvironmentHash,
}

/// Hash input for H4 Phase-A cleanliness comparison.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PhaseAEffectiveConfigHashInput<'a> {
    optimizer_steps: u64,
    batch_size: usize,
    sequence_length: usize,
    eval_every_steps: u64,
    eval_subset_size: u64,
    optimizer: AdamWConfig,
    phase_plan: &'a [TrainConfigS2Phase],
    rng_kind: RngKind,
    device_profile: &'a S1CpuDeterministic,
}

/// Compute the full S2 train-config hash for a runtime build kind.
pub fn train_config_hash(
    config: &TrainConfigS2Full,
    build_kind: S2BuildKind,
) -> Result<Hash256, S1SchemaError> {
    train_config_hash_with_environment_hash(
        config,
        build_kind,
        crate::s2::environment::compute_environment_hash()?,
    )
}

/// Compute the full S2 train-config hash with an explicit environment hash.
///
/// This keeps production hashing on [`crate::s2::environment::compute_environment_hash`]
/// while giving tests a direct, uncached proof that environment mutations perturb
/// the train-config hash input.
pub fn train_config_hash_with_environment_hash(
    config: &TrainConfigS2Full,
    build_kind: S2BuildKind,
    environment_hash: S2EnvironmentHash,
) -> Result<Hash256, S1SchemaError> {
    config
        .validate()
        .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
    let input = TrainConfigS2HashInput {
        config,
        build_kind,
        quant_hardness_override: quant_hardness_override_for_build_kind(build_kind),
        lambda_distill_effective_default: lambda_distill_default_for_build_kind(
            config.lambda_distill_default,
            build_kind,
        ),
        environment_hash,
    };
    let hash = DomainHash::new(
        "gbf-experiments",
        "TrainConfigS2Full",
        train_config_schema_id(build_kind),
        "1",
    )
    .hash(&input)?;
    let phase_a_effective_hash = phase_a_effective_config_hash(config)?;
    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "train_config_built",
        build_kind = ?build_kind,
        train_config_hash = %hash,
        phase_a_effective_hash = %phase_a_effective_hash,
        training_loss_unit = "nats",
        "s2 train config hash built"
    );
    Ok(hash)
}

/// Compute the Phase-A-effective hash used only by H4 ablation comparison.
pub fn phase_a_effective_config_hash(config: &TrainConfigS2Full) -> Result<Hash256, S1SchemaError> {
    config
        .validate()
        .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
    let phase_a = phase_a_plan();
    let input = PhaseAEffectiveConfigHashInput {
        optimizer_steps: S2_TEACHER_FREEZE_STEP,
        batch_size: config.batch_size,
        sequence_length: config.sequence_length,
        eval_every_steps: config.eval_every_steps,
        eval_subset_size: config.eval_subset_size,
        optimizer: config.optimizer,
        phase_plan: &phase_a,
        rng_kind: config.rng_kind,
        device_profile: &config.device_profile,
    };
    DomainHash::new(
        "gbf-experiments",
        "TrainConfigS2PhaseAOnly",
        "s2_phase_a_effective_config.v1",
        "1",
    )
    .hash(&input)
}

/// Return the runtime hardness override implied by a build kind.
#[must_use]
pub const fn quant_hardness_override_for_build_kind(
    build_kind: S2BuildKind,
) -> QuantHardnessOverride {
    match build_kind {
        S2BuildKind::s2_fp_full => QuantHardnessOverride::AllOff,
        S2BuildKind::s2_ternary_full
        | S2BuildKind::s2_ternary_nodistill
        | S2BuildKind::s2_ablation => QuantHardnessOverride::None,
    }
}

/// Return the effective default distillation lambda implied by a build kind.
#[must_use]
pub const fn lambda_distill_default_for_build_kind(
    configured_default: f32,
    build_kind: S2BuildKind,
) -> f32 {
    match build_kind {
        S2BuildKind::s2_ternary_nodistill => 0.0,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_fp_full | S2BuildKind::s2_ablation => {
            configured_default
        }
    }
}

/// S2 hypothesis status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum HypothesisStatus {
    /// Hypothesis confirmed.
    Confirmed,
    /// Hypothesis refuted.
    Refuted,
    /// Hypothesis was not evaluated because an earlier gate stopped.
    NotEvaluatedDueToPriorGate {
        /// Human-readable prior gate reason.
        reason: String,
    },
}

impl HypothesisStatus {
    /// True when the hypothesis reached a binary closure verdict.
    #[must_use]
    pub const fn is_binary_closure_verdict(&self) -> bool {
        matches!(self, Self::Confirmed | Self::Refuted)
    }
}

/// Verifier/report failure classes for S2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailureKindS2 {
    /// Substrate failure.
    Substrate,
    /// Ternary-vs-fp gap failure.
    Gap,
    /// Distillation failure.
    Distill,
    /// Phase schedule failure.
    Phase,
    /// Measurement-oracle failure.
    Metric,
    /// Suspicious but not definitively invalid run.
    Suspicious,
    /// LinearState gradient-smoke failure.
    LinearState,
    /// Loss-gradient-flow failure.
    LossGradFlow,
    /// Phase-transition integration failure.
    PhaseIntegration,
    /// Public API drift failure.
    ApiDrift,
    /// Pre-registration failure.
    Preregistration,
    /// Artifact failure.
    Artifact,
    /// Incomplete run or artifact set.
    Incomplete,
}

/// First mismatching tensor byte in an S2 ablation comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2TensorMismatch {
    /// Tensor identifier from the canonical tensor payload.
    pub tensor: String,
    /// Byte offset of the first mismatch inside that tensor payload.
    pub byte_offset: u64,
}

/// `s2_ablation.v1` seed-0 Phase A equality artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2AblationReport {
    /// Schema id. Expected value: `s2_ablation.v1`.
    pub schema: String,
    /// S2 seed; the closure comparison is seed 0.
    pub seed: u64,
    /// Ternary-full Phase A checkpoint hash.
    pub s2_ternary_phase_a_checkpoint_sha: Hash256,
    /// Ablation Phase A checkpoint hash.
    pub s2_ablation_phase_a_checkpoint_sha: Hash256,
    /// Canonical tensor payload hash for the ternary-full Phase A checkpoint.
    pub s2_ternary_tensor_payload_sha: Hash256,
    /// Canonical tensor payload hash for the ablation Phase A checkpoint.
    pub s2_ablation_tensor_payload_sha: Hash256,
    /// Whether the two tensor payloads are byte-identical.
    pub phase_a_eq_ablation: bool,
    /// First mismatch when payloads differ.
    pub first_mismatch: Option<S2TensorMismatch>,
    /// Self-hash over this artifact with this field omitted.
    pub ablation_self_hash: Hash256,
}

impl S2AblationReport {
    /// Construct and hash an `s2_ablation.v1` report.
    pub fn new(
        seed: u64,
        s2_ternary_phase_a_checkpoint_sha: Hash256,
        s2_ablation_phase_a_checkpoint_sha: Hash256,
        s2_ternary_tensor_payload_sha: Hash256,
        s2_ablation_tensor_payload_sha: Hash256,
        first_mismatch: Option<S2TensorMismatch>,
    ) -> Result<Self, S1SchemaError> {
        let phase_a_eq_ablation = s2_ternary_tensor_payload_sha == s2_ablation_tensor_payload_sha;
        Self {
            schema: S2_ABLATION_SCHEMA.to_owned(),
            seed,
            s2_ternary_phase_a_checkpoint_sha,
            s2_ablation_phase_a_checkpoint_sha,
            s2_ternary_tensor_payload_sha,
            s2_ablation_tensor_payload_sha,
            phase_a_eq_ablation,
            first_mismatch,
            ablation_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate AB-1 structural consistency.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != S2_ABLATION_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "S2AblationReport schema must be s2_ablation.v1",
            ));
        }
        if self.seed != 0 {
            return Err(S2SchemaError::InvalidVerifierReport(
                "S2AblationReport seed must be 0",
            ));
        }
        let expected_eq = self.s2_ternary_tensor_payload_sha == self.s2_ablation_tensor_payload_sha;
        if self.phase_a_eq_ablation != expected_eq {
            return Err(S2SchemaError::InvalidVerifierReport(
                "AB-1 requires phase_a_eq_ablation to match tensor payload hash equality",
            ));
        }
        if self.phase_a_eq_ablation && self.first_mismatch.is_some() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "matching S2 ablation payloads must not record first_mismatch",
            ));
        }
        if !self.phase_a_eq_ablation && self.first_mismatch.is_none() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "mismatching S2 ablation payloads must record first_mismatch",
            ));
        }
        Ok(())
    }

    /// Validate AB-2 closure eligibility.
    pub fn validate_closure(&self) -> Result<(), S2SchemaError> {
        self.validate()?;
        if !self.phase_a_eq_ablation {
            return Err(S2SchemaError::InvalidVerifierReport(
                "AB-2 closure requires phase_a_eq_ablation=true",
            ));
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting the report self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "ablation_self_hash")
    }

    /// Compute the artifact self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact(
            "S2AblationReport",
            S2_ABLATION_SCHEMA,
            self,
            "ablation_self_hash",
        )
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.ablation_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// `s2_oracle_re_run.v1` measurement oracle re-run artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2OracleReRunReport {
    /// Schema id. Expected value: `s2_oracle_re_run.v1`.
    pub schema: String,
    /// S1 oracle suite version re-run under the S2 binary.
    pub s1_oracle_suite_version: String,
    /// Whether every oracle case passed.
    pub metric_oracle_passed: bool,
    /// Oracle case ids invoked by the re-run.
    pub oracle_cases: Vec<String>,
    /// Self-hash over this artifact with this field omitted.
    pub oracle_re_run_self_hash: Hash256,
}

impl S2OracleReRunReport {
    /// Construct and hash an `s2_oracle_re_run.v1` report.
    pub fn new(
        s1_oracle_suite_version: impl Into<String>,
        metric_oracle_passed: bool,
        oracle_cases: Vec<String>,
    ) -> Result<Self, S1SchemaError> {
        Self {
            schema: S2_ORACLE_RE_RUN_SCHEMA.to_owned(),
            s1_oracle_suite_version: s1_oracle_suite_version.into(),
            metric_oracle_passed,
            oracle_cases,
            oracle_re_run_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
    }

    /// Validate structural oracle re-run invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != S2_ORACLE_RE_RUN_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "S2OracleReRunReport schema must be s2_oracle_re_run.v1",
            ));
        }
        if self.s1_oracle_suite_version.trim().is_empty() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s1_oracle_suite_version must not be empty",
            ));
        }
        if self.s1_oracle_suite_version != crate::s2::oracle_re_run::S1_ORACLE_SUITE_VERSION {
            return Err(S2SchemaError::InvalidVerifierReport(
                "s1_oracle_suite_version must match S1_ORACLE_SUITE_VERSION",
            ));
        }
        if self.oracle_cases.is_empty() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "oracle_cases must not be empty",
            ));
        }
        if self.oracle_cases.iter().any(|case| case.trim().is_empty()) {
            return Err(S2SchemaError::InvalidVerifierReport(
                "oracle_cases must not contain empty ids",
            ));
        }
        if !self
            .oracle_cases
            .iter()
            .map(String::as_str)
            .eq(crate::s2::oracle_re_run::ORACLE_CASE_IDS.iter().copied())
        {
            return Err(S2SchemaError::InvalidVerifierReport(
                "oracle_cases must match S1 D7 O-metric case ids",
            ));
        }
        Ok(())
    }

    /// Validate OR-1 closure eligibility.
    pub fn validate_closure(&self) -> Result<(), S2SchemaError> {
        self.validate()?;
        if !self.metric_oracle_passed {
            return Err(S2SchemaError::InvalidVerifierReport(
                "OR-1 closure requires metric_oracle_passed=true",
            ));
        }
        Ok(())
    }

    /// Canonical JSON bytes omitting the report self-hash.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        canonical_json_bytes_omitting_self_hash(self, "oracle_re_run_self_hash")
    }

    /// Compute the artifact self-hash.
    pub fn computed_self_hash(&self) -> Result<Hash256, S1SchemaError> {
        self_hash_for_artifact(
            "S2OracleReRunReport",
            S2_ORACLE_RE_RUN_SCHEMA,
            self,
            "oracle_re_run_self_hash",
        )
    }

    /// Return a copy with the self-hash recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S1SchemaError> {
        self.validate()
            .map_err(|error| S1SchemaError::Custom(error.to_string()))?;
        self.oracle_re_run_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// S2 report outcome tag from RFC section 8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum S2Outcome {
    /// H1 through H6 all confirmed.
    #[serde(rename = "Pass-clean")]
    PassClean,
    /// Distillation control refuted while other closure hypotheses confirmed.
    #[serde(rename = "Pass-with-distill-warn")]
    PassWithDistillWarn,
    /// H1 refuted, or any seed diverged.
    #[serde(rename = "Fail-substrate")]
    FailSubstrate,
    /// H2 refuted without suspicious low-bpc evidence.
    #[serde(rename = "Fail-gap")]
    FailGap,
    /// Median bpc is suspiciously low.
    #[serde(rename = "Fail-suspicious")]
    FailSuspicious,
    /// H4 ablation/phase cleanliness refuted.
    #[serde(rename = "Fail-phase")]
    FailPhase,
    /// H5 loss gradient-flow refuted.
    #[serde(rename = "Fail-loss-grad-flow")]
    FailLossGradFlow,
    /// H6 LinearState smoke refuted.
    #[serde(rename = "Fail-linearstate")]
    FailLinearstate,
    /// Phase-transition integration failed.
    #[serde(rename = "Fail-phase-integration")]
    FailPhaseIntegration,
    /// S2 falsification suite failed.
    #[serde(rename = "Fail-falsification")]
    FailFalsification,
    /// Public API drift check failed.
    #[serde(rename = "Fail-api-drift")]
    FailApiDrift,
    /// Measurement oracle regressed under the S2 binary.
    #[serde(rename = "Fail-metric")]
    FailMetric,
    /// Pre-registration proof failed.
    #[serde(rename = "Fail-preregistration")]
    FailPreregistration,
    /// Required artifact was missing or self-hash invalid.
    #[serde(rename = "Fail-artifact")]
    FailArtifact,
    /// A required non-gating artifact was missing.
    #[serde(rename = "Fail-incomplete")]
    FailIncomplete,
}

impl S2Outcome {
    /// All S2 report outcome tags currently accepted by the schema.
    pub const ALL: [Self; 15] = [
        Self::PassClean,
        Self::PassWithDistillWarn,
        Self::FailSubstrate,
        Self::FailGap,
        Self::FailSuspicious,
        Self::FailPhase,
        Self::FailLossGradFlow,
        Self::FailLinearstate,
        Self::FailPhaseIntegration,
        Self::FailFalsification,
        Self::FailApiDrift,
        Self::FailMetric,
        Self::FailPreregistration,
        Self::FailArtifact,
        Self::FailIncomplete,
    ];
}

/// S2 decision tag from RFC section 8.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum S2Decision {
    /// Proceed to S3 without warnings.
    #[serde(rename = "ProceedToS3")]
    ProceedToS3,
    /// Proceed to S3 with a required distillation review.
    #[serde(rename = "ProceedToS3-with-distill-review")]
    ProceedToS3WithDistillReview,
    /// Investigation is required before S2 can close.
    #[serde(rename = "Investigate")]
    Investigate {
        /// Investigation reason tag.
        reason: String,
    },
    /// Halt blocks closure.
    #[serde(rename = "Halt")]
    Halt {
        /// Halt reason tag.
        reason: String,
    },
}

/// Verifier and early-gate inputs consumed by the S2 outcome dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2VerifierBundle {
    /// Pre-registration proof gate.
    pub preregistration_passed: bool,
    /// Required artifact presence and self-hash gate.
    pub artifact_integrity_passed: bool,
    /// S1 oracle re-run under the S2 binary.
    pub oracle_re_run_passed: bool,
    /// Public API drift checker result.
    pub api_drift_check_passed: bool,
    /// Falsification suite result.
    pub falsification_s2_passed: bool,
    /// D8 phase-transition integration result.
    pub phase_transition_integ_passed: bool,
    /// Whether all methodological controls needed by later hypotheses exist.
    pub methodological_controls_present: bool,
    /// Suspicious-low median bpc sentinel for any scored build.
    pub suspicious_low_bpc: bool,
    /// Per-seed completion states across the S2 build matrix.
    pub completions: Vec<S2Completion>,
    /// Explicit verdict status for all six hypotheses.
    pub hypothesis_statuses: BTreeMap<S2Hypothesis, HypothesisStatus>,
}

impl S2VerifierBundle {
    /// Construct a closure-candidate bundle with all gates and hypotheses passing.
    #[must_use]
    pub fn closure_candidate() -> Self {
        Self {
            preregistration_passed: true,
            artifact_integrity_passed: true,
            oracle_re_run_passed: true,
            api_drift_check_passed: true,
            falsification_s2_passed: true,
            phase_transition_integ_passed: true,
            methodological_controls_present: true,
            suspicious_low_bpc: false,
            completions: vec![S2Completion::Completed; 15],
            hypothesis_statuses: all_confirmed_hypotheses(),
        }
    }

    /// Return the status for one S2 hypothesis.
    #[must_use]
    pub fn status(&self, hypothesis: S2Hypothesis) -> HypothesisStatus {
        self.hypothesis_statuses
            .get(&hypothesis)
            .cloned()
            .unwrap_or_else(|| HypothesisStatus::NotEvaluatedDueToPriorGate {
                reason: "missing hypothesis status".to_owned(),
            })
    }

    /// True if any seed/build diverged.
    #[must_use]
    pub fn any_seed_diverged(&self) -> bool {
        self.completions
            .iter()
            .any(|completion| matches!(completion, S2Completion::DivergedAt { .. }))
    }

    /// True if any seed/build was not reached.
    #[must_use]
    pub fn any_not_reached(&self) -> bool {
        self.completions
            .iter()
            .any(|completion| matches!(completion, S2Completion::NotReached))
    }

    /// Return the first not-evaluated hypothesis status, if any.
    #[must_use]
    pub fn first_not_evaluated(&self) -> Option<(S2Hypothesis, HypothesisStatus)> {
        S2Hypothesis::ALL.into_iter().find_map(|hypothesis| {
            let status = self.status(hypothesis);
            (!status.is_binary_closure_verdict()).then_some((hypothesis, status))
        })
    }
}

fn all_confirmed_hypotheses() -> BTreeMap<S2Hypothesis, HypothesisStatus> {
    S2Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

/// One of the six F-S2 hypotheses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S2Hypothesis {
    /// H1 substrate survival.
    H1,
    /// H2 ternary-vs-fp bpc gap.
    H2,
    /// H3 distillation contribution.
    H3,
    /// H4 Phase A ablation cleanliness.
    H4,
    /// H5 loss gradient-flow.
    H5,
    /// H6 LinearState gradient smoke.
    H6,
}

impl S2Hypothesis {
    /// All six S2 hypotheses in canonical closure order.
    pub const ALL: [Self; 6] = [Self::H1, Self::H2, Self::H3, Self::H4, Self::H5, Self::H6];
}

/// Completion state recorded in `s2_report.v1` per-seed rows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum S2Completion {
    /// The run completed its requested optimizer steps.
    Completed,
    /// The run observed divergence at the recorded train step.
    DivergedAt {
        /// First diverged global train step.
        step: GlobalStep,
    },
    /// The run product was not reached because an earlier gate stopped.
    NotReached,
}

/// Per-phase checkpoint self-hashes carried by `s2_report.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2CheckpointSelfHashes {
    /// Phase A checkpoint self-hash.
    pub phase_a: Option<Hash256>,
    /// Phase B checkpoint self-hash.
    pub phase_b: Option<Hash256>,
    /// Phase C checkpoint self-hash.
    pub phase_c: Option<Hash256>,
    /// Final checkpoint self-hash.
    #[serde(rename = "final")]
    pub final_checkpoint: Option<Hash256>,
}

impl S2CheckpointSelfHashes {
    /// Return true when every checkpoint hash required for a closure row exists.
    #[must_use]
    pub fn all_present(&self) -> bool {
        self.phase_a.is_some()
            && self.phase_b.is_some()
            && self.phase_c.is_some()
            && self.final_checkpoint.is_some()
    }
}

/// Per-seed S2 report artifact references.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2PerSeedArtifacts {
    /// Seed id.
    pub seed: u64,
    /// Runtime build kind.
    pub build_kind: S2BuildKind,
    /// Completion state for this seed/build row.
    pub completion: S2Completion,
    /// Per-phase checkpoint self-hashes.
    pub checkpoint_self_hashes: S2CheckpointSelfHashes,
    /// Phase-log self-hash.
    pub phase_log_self_hash: Option<Hash256>,
    /// Score-report self-hash.
    pub score_self_hash: Option<Hash256>,
    /// Distillation log self-hash.
    pub distill_log_self_hash: Option<Hash256>,
}

/// `s2_report.v1` front matter. The markdown body is owned by the report emitter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S2ReportFrontMatter {
    /// Schema id. Expected value: `s2_report.v1`.
    pub schema: String,
    /// S2 outcome tag.
    pub s2_outcome: S2Outcome,
    /// S2 decision tag.
    pub decision: S2Decision,
    /// S1 baseline self-hash carried forward for comparison.
    pub baseline_self_hash_carried_from_s1: Hash256,
    /// Whether the S1 oracle suite re-run passed under the S2 binary.
    pub oracle_re_run_passed: bool,
    /// Oracle re-run report self-hash.
    pub oracle_re_run_self_hash: Hash256,
    /// Whether the public API drift check passed.
    pub api_drift_check_passed: bool,
    /// Public QAT API snapshot hash.
    pub qat_public_api_snapshot_hash: Hash256,
    /// Public LinearState API snapshot hash.
    pub linearstate_public_api_snapshot_hash: Hash256,
    /// Per-seed S2 artifact references.
    pub per_seed_artifacts: Vec<S2PerSeedArtifacts>,
    /// S2 ablation report self-hash.
    pub ablation_self_hash: Option<Hash256>,
    /// Loss gradient-flow verifier self-hash.
    pub loss_grad_flow_self_hash: Hash256,
    /// LinearState smoke verifier self-hash.
    pub linearstate_smoke_self_hash: Hash256,
    /// Phase-transition integration verifier self-hash.
    pub phase_transition_integ_self_hash: Hash256,
    /// Whether the phase-transition integration verifier passed.
    pub phase_transition_integ_passed: bool,
    /// Whether the S2 falsification suite passed.
    pub falsification_s2_passed: bool,
    /// S2 falsification suite hash.
    pub falsification_s2_suite_hash: Hash256,
    /// RFC3339 UTC generation time. Excluded from `report_self_hash`.
    pub generated_at: String,
    /// RFC revision used for the report.
    pub rfc_revision: RfcRevisionRef,
    /// Hash of the pre-registered predictions section.
    pub predictions_section_hash: Hash256,
    /// Commit introducing the predictions section.
    pub predictions_commit: GitCommitId,
    /// First commit that introduced any S2 result artifact.
    pub first_result_commit: GitCommitId,
    /// Explicit verdict status for all six S2 hypotheses.
    pub hypothesis_statuses: BTreeMap<S2Hypothesis, HypothesisStatus>,
    /// S2 pass implementation version.
    #[serde(rename = "pass_version_S2")]
    pub pass_version_s2: SemVer,
    /// Self-hash over front matter with `generated_at` and this field omitted,
    /// plus the exact markdown body bytes.
    pub report_self_hash: Hash256,
}

impl S2ReportFrontMatter {
    /// DomainHash context for `s2_report.v1` front matter.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "S2ReportFrontMatter",
            S2_REPORT_SCHEMA,
            "1",
        )
    }

    /// Canonical JSON bytes for the front-matter portion of `s2_report.v1`.
    ///
    /// This omits `generated_at` and `report_self_hash`; callers that validate
    /// the final report must append the exact markdown body bytes via
    /// `crate::s2::report::report_self_hash`.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, S1SchemaError> {
        let mut value = serde_json::to_value(self).map_err(S1SchemaError::Json)?;
        let object = value
            .as_object_mut()
            .ok_or(S1SchemaError::ExpectedObjectForSelfHash)?;
        object.remove("generated_at");
        object.remove("report_self_hash");
        S1CanonicalJson::value_to_vec(&value)
    }

    /// Validate schema-level front-matter invariants.
    pub fn validate(&self) -> Result<(), S2SchemaError> {
        if self.schema != S2_REPORT_SCHEMA {
            return Err(S2SchemaError::InvalidVerifierReport(
                "S2ReportFrontMatter schema must be s2_report.v1",
            ));
        }
        if self.generated_at.trim().is_empty() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "generated_at must not be empty",
            ));
        }
        Ok(())
    }
}

/// Errors from S2 schema constructors.
#[derive(Debug, Clone, PartialEq)]
pub enum S2SchemaError {
    /// A finite f32 field was NaN or infinite.
    NonFiniteF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// A non-negative f32 field was negative.
    NegativeF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// A positive-only f32 was zero or negative.
    NonPositiveF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// Range bounds were invalid.
    InvalidRangeBounds {
        /// Lower bound.
        lo: f32,
        /// Upper bound.
        hi: f32,
    },
    /// Phase plan does not match the S2 contract.
    InvalidPhasePlan(&'static str),
    /// Verifier report invariant failed.
    InvalidVerifierReport(&'static str),
}

impl fmt::Display for S2SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteF32 { field, value } => {
                write!(f, "{field} must be finite, got {value}")
            }
            Self::NegativeF32 { field, value } => {
                write!(f, "{field} must be non-negative, got {value}")
            }
            Self::NonPositiveF32 { field, value } => {
                write!(f, "{field} must be positive, got {value}")
            }
            Self::InvalidRangeBounds { lo, hi } => {
                write!(
                    f,
                    "range_safe_lo must be <= range_safe_hi, got [{lo}, {hi}]"
                )
            }
            Self::InvalidPhasePlan(message) => f.write_str(message),
            Self::InvalidVerifierReport(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for S2SchemaError {}

fn optional_finite_nonnegative_f32<'de, D>(deserializer: D) -> Result<Option<f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<f32>::deserialize(deserializer)?;
    if let Some(value) = value {
        if !value.is_finite() {
            return Err(serde::de::Error::custom(
                "S2 optional f32 values must be finite",
            ));
        }
        if value < 0.0 {
            return Err(serde::de::Error::custom(
                "S2 optional f32 values must be non-negative",
            ));
        }
    }
    Ok(value)
}

fn finite_nonnegative_f32_map<'de, D>(deserializer: D) -> Result<BTreeMap<String, f32>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = BTreeMap::<String, f32>::deserialize(deserializer)?;
    for (name, value) in &values {
        if !value.is_finite() {
            return Err(serde::de::Error::custom(format!(
                "S2 map value {name:?} must be finite"
            )));
        }
        if *value < 0.0 {
            return Err(serde::de::Error::custom(format!(
                "S2 map value {name:?} must be non-negative"
            )));
        }
    }
    Ok(values)
}

fn finite_nonnegative_f32<'de, D>(deserializer: D) -> Result<f32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = f32::deserialize(deserializer)?;
    if !value.is_finite() {
        return Err(serde::de::Error::custom(
            "S2 phase-effective lambdas must be finite",
        ));
    }
    if value < 0.0 {
        return Err(serde::de::Error::custom(
            "S2 phase-effective lambdas must be non-negative",
        ));
    }
    Ok(value)
}

fn validate_finite_nonnegative(field: &'static str, value: f32) -> Result<(), S2SchemaError> {
    validate_finite(field, value)?;
    if value < 0.0 {
        return Err(S2SchemaError::NegativeF32 { field, value });
    }
    Ok(())
}

fn validate_finite_positive(field: &'static str, value: f32) -> Result<(), S2SchemaError> {
    validate_finite(field, value)?;
    if value <= 0.0 {
        return Err(S2SchemaError::NonPositiveF32 { field, value });
    }
    Ok(())
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), S2SchemaError> {
    if !value.is_finite() {
        return Err(S2SchemaError::NonFiniteF32 { field, value });
    }
    Ok(())
}

fn validate_finite_nonnegative_f64(field: &'static str, value: f64) -> Result<(), S2SchemaError> {
    if !value.is_finite() || value < 0.0 {
        return Err(S2SchemaError::InvalidVerifierReport(match field {
            "log2_sum" => "log2_sum must be finite and non-negative",
            "bpc" => "bpc must be finite and non-negative",
            _ => "f64 diagnostic must be finite and non-negative",
        }));
    }
    Ok(())
}

fn validate_finite_nonnegative_map_value(
    field: &'static str,
    name: &str,
    value: f32,
) -> Result<(), S2SchemaError> {
    if !value.is_finite() || value < 0.0 {
        return Err(S2SchemaError::InvalidVerifierReport(match field {
            "in_scope_grad_norms" => "in_scope_grad_norms must be finite and non-negative",
            "stop_gradient_grad_norms" => {
                "stop_gradient_grad_norms must be finite and non-negative"
            }
            "param_grad_norms" => "param_grad_norms must be finite and non-negative",
            _ => {
                let _ = name;
                "S2 verifier map values must be finite and non-negative"
            }
        }));
    }
    Ok(())
}

fn expected_phase_log_steps(build_kind: S2BuildKind) -> u64 {
    if build_kind == S2BuildKind::s2_ablation {
        S2_TEACHER_FREEZE_STEP
    } else {
        S2_OPTIMIZER_STEPS
    }
}

fn validate_phase_entry_for_build(
    entry: &PhaseEntry,
    build_kind: S2BuildKind,
) -> Result<(), S2SchemaError> {
    let expected_phase = expected_phase_for_step(entry.step, build_kind)?;
    if entry.phase != expected_phase {
        return Err(S2SchemaError::InvalidVerifierReport(
            "PhaseEntry phase must match D1 schedule",
        ));
    }
    if entry.hardness != expected_hardness_for_step(entry.step, build_kind)? {
        return Err(S2SchemaError::InvalidVerifierReport(
            "PL-4 hardness must match D2 schedule and build override",
        ));
    }
    let expected_teacher_frozen =
        build_kind != S2BuildKind::s2_ablation && entry.step > S2_TEACHER_FREEZE_STEP;
    if entry.teacher_frozen != expected_teacher_frozen {
        return Err(S2SchemaError::InvalidVerifierReport(
            "PL-3 teacher_frozen must flip after step 4000",
        ));
    }
    let pre_phase_c = entry.step <= S2_PHASE_B_END_STEP;
    if build_kind == S2BuildKind::s2_ablation || pre_phase_c {
        if entry.distill_loss.is_some() {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PL-5 distill_loss must be null before Phase C and for ablation",
            ));
        }
    } else if entry.distill_loss.is_none() {
        return Err(S2SchemaError::InvalidVerifierReport(
            "PL-5 distill_loss must be finite in Phase C/D full builds",
        ));
    }
    Ok(())
}

fn expected_phase_for_step(
    step: GlobalStep,
    build_kind: S2BuildKind,
) -> Result<PhaseKindS2, S2SchemaError> {
    if build_kind == S2BuildKind::s2_ablation {
        if (1..=S2_TEACHER_FREEZE_STEP).contains(&step) {
            return Ok(PhaseKindS2::PhaseA);
        }
    } else if (1..=S2_TEACHER_FREEZE_STEP).contains(&step) {
        return Ok(PhaseKindS2::PhaseA);
    } else if (S2_TEACHER_FREEZE_STEP + 1..=S2_PHASE_B_END_STEP).contains(&step) {
        return Ok(PhaseKindS2::PhaseB);
    } else if (S2_PHASE_B_END_STEP + 1..=S2_PHASE_C_END_STEP).contains(&step) {
        return Ok(PhaseKindS2::PhaseC);
    } else if (S2_PHASE_C_END_STEP + 1..=S2_OPTIMIZER_STEPS).contains(&step) {
        return Ok(PhaseKindS2::PhaseD);
    }
    Err(S2SchemaError::InvalidVerifierReport(
        "PhaseEntry step is outside build phase range",
    ))
}

fn expected_hardness_for_step(
    step: GlobalStep,
    build_kind: S2BuildKind,
) -> Result<HardnessTriple, S2SchemaError> {
    if matches!(
        build_kind,
        S2BuildKind::s2_fp_full | S2BuildKind::s2_ablation
    ) {
        return Ok(HardnessTriple::all_off());
    }
    let phase = expected_phase_for_step(step, build_kind)?;
    let local_step = match phase {
        PhaseKindS2::PhaseA => step,
        PhaseKindS2::PhaseB => step - S2_TEACHER_FREEZE_STEP,
        PhaseKindS2::PhaseC => step - S2_PHASE_B_END_STEP,
        PhaseKindS2::PhaseD => step - S2_PHASE_C_END_STEP,
    };
    Ok(match phase {
        PhaseKindS2::PhaseA | PhaseKindS2::PhaseB => HardnessTriple::all_off(),
        PhaseKindS2::PhaseC if local_step <= 1_000 => HardnessTriple::all_off(),
        PhaseKindS2::PhaseC if local_step <= 2_000 => {
            HardnessTriple::new(QuantHardness::Soft, QuantHardness::Off, QuantHardness::Off)
        }
        PhaseKindS2::PhaseC => {
            HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off)
        }
        PhaseKindS2::PhaseD if local_step <= 500 => {
            HardnessTriple::new(QuantHardness::Hard, QuantHardness::Off, QuantHardness::Off)
        }
        PhaseKindS2::PhaseD if local_step <= 1_000 => HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Soft,
            QuantHardness::Soft,
        ),
        PhaseKindS2::PhaseD => HardnessTriple::new(
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
        ),
    })
}

fn matches_phase_transition(step: u64, from: PhaseKindS2, to: PhaseKindS2) -> bool {
    matches!(
        (step, from, to),
        (4_001, PhaseKindS2::PhaseA, PhaseKindS2::PhaseB)
            | (5_001, PhaseKindS2::PhaseB, PhaseKindS2::PhaseC)
            | (8_001, PhaseKindS2::PhaseC, PhaseKindS2::PhaseD)
    )
}

fn count_phase_transitions(entries: &[PhaseEntry]) -> u32 {
    entries
        .iter()
        .flat_map(|entry| &entry.events)
        .filter(|event| matches!(event, PhaseEvent::PhaseTransition { .. }))
        .count() as u32
}

fn count_teacher_freezes(entries: &[PhaseEntry]) -> u32 {
    entries
        .iter()
        .flat_map(|entry| &entry.events)
        .filter(|event| matches!(event, PhaseEvent::TeacherFreeze { .. }))
        .count() as u32
}

fn self_hash_for_artifact<T>(
    type_name: &'static str,
    schema_id: &'static str,
    artifact: &T,
    self_hash_field: &str,
) -> Result<Hash256, S1SchemaError>
where
    T: Serialize,
{
    let value = serde_json::to_value(artifact).map_err(S1SchemaError::Json)?;
    self_hash_for_value(
        DomainHash::new("gbf-experiments", type_name, schema_id, "1"),
        &value,
        self_hash_field,
    )
}

fn canonical_json_bytes_omitting_self_hash<T>(
    artifact: &T,
    self_hash_field: &str,
) -> Result<Vec<u8>, S1SchemaError>
where
    T: Serialize,
{
    let mut value = serde_json::to_value(artifact).map_err(S1SchemaError::Json)?;
    value
        .as_object_mut()
        .ok_or(S1SchemaError::ExpectedObjectForSelfHash)?
        .remove(self_hash_field);
    S1CanonicalJson::value_to_vec(&value)
}

fn canonical_fixture_phase_boundaries() -> Vec<u32> {
    vec![0, 10, 20, 30, 40, 50]
}

fn parsed_hardness_at_boundary(
    hardness_at_boundary: &BTreeMap<String, HardnessTriple>,
) -> Result<BTreeMap<u32, &HardnessTriple>, S2SchemaError> {
    let mut parsed = BTreeMap::new();
    for (boundary, hardness) in hardness_at_boundary {
        let parsed_boundary = boundary
            .parse::<u32>()
            .map_err(|_| S2SchemaError::InvalidVerifierReport("PT-3 boundary keys must be u32"))?;
        if parsed_boundary.to_string() != *boundary {
            return Err(S2SchemaError::InvalidVerifierReport(
                "PT-3 boundary keys must be canonical u32 strings",
            ));
        }
        parsed.insert(parsed_boundary, hardness);
    }
    Ok(parsed)
}

fn first_mismatched_phase_boundary(
    hardness_at_boundary: &BTreeMap<String, HardnessTriple>,
) -> Option<u64> {
    for (boundary, expected) in phase_transition_expected_hardness_at_boundary() {
        match hardness_at_boundary.get(&boundary) {
            Some(observed) if *observed == expected => {}
            _ => {
                return boundary.parse::<u64>().ok();
            }
        }
    }
    hardness_at_boundary
        .keys()
        .find_map(|boundary| boundary.parse::<u64>().ok())
}

fn log_linearstate_invariant_failed(
    ls_id: &'static str,
    parameter: Option<&str>,
    observed: serde_json::Value,
) {
    tracing::error!(
        target: S2_LOG_TARGET,
        event_name = "linearstate_smoke_invariant_failed",
        ls_id = ls_id,
        parameter = parameter.unwrap_or("null"),
        observed = %observed,
        "s2 linearstate smoke invariant failed"
    );
}

fn log_phase_transition_invariant_failed(
    pt_id: &'static str,
    boundary: Option<u64>,
    observed: serde_json::Value,
) {
    tracing::error!(
        target: S2_LOG_TARGET,
        event_name = "phase_transition_integ_invariant_failed",
        pt_id = pt_id,
        boundary = boundary.unwrap_or(0),
        observed = %observed,
        "s2 phase-transition integration invariant failed"
    );
}

fn full_phase_plan() -> Vec<TrainConfigS2Phase> {
    vec![
        TrainConfigS2Phase::new(PhaseKindS2::PhaseA, 1, S2_TEACHER_FREEZE_STEP)
            .expect("canonical Phase A range is valid"),
        TrainConfigS2Phase::new(
            PhaseKindS2::PhaseB,
            S2_TEACHER_FREEZE_STEP + 1,
            S2_PHASE_B_END_STEP,
        )
        .expect("canonical Phase B range is valid"),
        TrainConfigS2Phase::new(
            PhaseKindS2::PhaseC,
            S2_PHASE_B_END_STEP + 1,
            S2_PHASE_C_END_STEP,
        )
        .expect("canonical Phase C range is valid"),
        TrainConfigS2Phase::new(
            PhaseKindS2::PhaseD,
            S2_PHASE_C_END_STEP + 1,
            S2_OPTIMIZER_STEPS,
        )
        .expect("canonical Phase D range is valid"),
    ]
}

fn phase_a_plan() -> Vec<TrainConfigS2Phase> {
    vec![
        TrainConfigS2Phase::new(PhaseKindS2::PhaseA, 1, S2_TEACHER_FREEZE_STEP)
            .expect("canonical Phase A range is valid"),
    ]
}

fn train_config_schema_id(build_kind: S2BuildKind) -> &'static str {
    match build_kind {
        S2BuildKind::s2_ternary_full => "s2_train_config.v1/s2_ternary_full",
        S2BuildKind::s2_fp_full => "s2_train_config.v1/s2_fp_full",
        S2BuildKind::s2_ternary_nodistill => "s2_train_config.v1/s2_ternary_nodistill",
        S2BuildKind::s2_ablation => "s2_train_config.v1/s2_ablation",
    }
}
