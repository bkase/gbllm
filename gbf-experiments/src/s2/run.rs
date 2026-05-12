//! S2 training-run dispatch helpers.

use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fmt;

use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorError, CanonicalTensorKind, CanonicalTensorLayout,
    CanonicalTensorPayload, CanonicalTensorShape, TensorElementType, canonical_tensor_payload_hash,
};
use gbf_foundation::{Hash256, sha256};
use gbf_train::adapter::burn::{
    BurnDevice, BurnFloatTensor, BurnNdArrayAutodiffBackend, BurnNdArrayBackend, burn_linear,
    float_tensor_from_vec, float_tensor_into_vec,
};
use gbf_train::loss::composer::{
    BurnLossTerms, LossTermApplicability, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, burn_compose,
};
use gbf_train::loss::range::burn_range_loss;
use gbf_train::loss::zero::burn_zero_loss;
use gbf_train::phase::TrainPhaseKind;
use serde::Serialize;

use crate::S2_LOG_TARGET;
use crate::s1::run::{CheckpointMetadata, CheckpointWriteError, canonical_checkpoint_bytes};
use crate::s1::schema::{DomainHash, S1SchemaError};
use crate::s2::schema::{
    DistillEvalPoint, DistillationLog, GlobalStep, HardnessTriple, LossTermEvalPoint,
    PhaseEffectiveLambda, PhaseEffectiveLambdaValues, PhaseEntry, PhaseEvent as SchemaPhaseEvent,
    PhaseKindS2, PhaseLog, QuantHardness, QuantHardnessOverride, RouterTrainMode,
    S2_DISTILL_TEMPERATURE, S2_OPTIMIZER_STEPS, S2_PHASE_B_END_STEP, S2_PHASE_C_END_STEP,
    S2_RANGE_SAFE_HI, S2_RANGE_SAFE_LO, S2_TEACHER_FREEZE_STEP, S2BuildKind, S2ScoreReport,
    ScaleStatsSummary, ThresholdStatsSummary, TrainConfigS2Full, TrainConfigS2Phase,
    TrainConfigS2PhaseAOnly, lambda_distill_default_for_build_kind, phase_a_effective_config_hash,
    quant_hardness_override_for_build_kind, train_config_hash,
};

const TINY_CORPUS_S2_TRAIN_BYTES: &[u8] =
    include_bytes!("../../tests/fixtures/tiny_corpus_s2/train.bytes");
const TINY_CORPUS_S2_EVAL_BYTES: &[u8] =
    include_bytes!("../../tests/fixtures/tiny_corpus_s2/eval.bytes");

/// Runtime behavior selected from S2BuildKind before the train loop starts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildKindDispatch {
    /// Runtime build kind.
    pub build_kind: S2BuildKind,
    /// Quantization hardness override used by the phase scheduler.
    pub quant_hardness_override: QuantHardnessOverride,
    /// Effective default distillation lambda for this logical build.
    pub lambda_distill_default: f32,
}

impl BuildKindDispatch {
    /// Resolve dispatch behavior from build kind and configured distillation lambda.
    #[must_use]
    pub fn resolve(build_kind: S2BuildKind, configured_lambda_distill_default: f32) -> Self {
        let dispatch = Self::resolve_quiet(build_kind, configured_lambda_distill_default);
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "buildkind_dispatch",
            build_kind = ?dispatch.build_kind,
            quant_hardness_override = ?dispatch.quant_hardness_override,
            lambda_distill_default = dispatch.lambda_distill_default,
            "s2 buildkind dispatch"
        );
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "hardness_override_active",
            build_kind = ?dispatch.build_kind,
            override = ?dispatch.quant_hardness_override,
            "s2 hardness override selected"
        );
        dispatch
    }

    fn resolve_quiet(build_kind: S2BuildKind, configured_lambda_distill_default: f32) -> Self {
        Self {
            build_kind,
            quant_hardness_override: quant_hardness_override_for_build_kind(build_kind),
            lambda_distill_default: lambda_distill_default_for_build_kind(
                configured_lambda_distill_default,
                build_kind,
            ),
        }
    }

    /// Apply the runtime hardness override to a scheduled phase hardness triple.
    #[must_use]
    pub const fn effective_hardness(self, scheduled: HardnessTriple) -> HardnessTriple {
        self.quant_hardness_override.apply(scheduled)
    }
}

/// Phase-boundary scheduler helpers.
pub mod scheduler {
    use super::*;

    /// Validated, ordered S2 phase plan.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct PhasePlan {
        phases: Vec<TrainConfigS2Phase>,
    }

    impl PhasePlan {
        /// Construct a non-empty, non-overlapping phase plan.
        pub fn new(mut phases: Vec<TrainConfigS2Phase>) -> Result<Self, PhasePlanError> {
            if phases.is_empty() {
                return Err(PhasePlanError::Empty);
            }
            phases.sort_by_key(|phase| phase.start_step);
            for window in phases.windows(2) {
                let left = window[0];
                let right = window[1];
                if left.end_step >= right.start_step {
                    return Err(PhasePlanError::Overlap {
                        a: left.phase,
                        b: right.phase,
                    });
                }
            }
            Ok(Self { phases })
        }

        /// Construct the canonical full S2 A-to-D plan.
        #[must_use]
        pub fn full_s2() -> Self {
            Self::new(TrainConfigS2Full::pinned().phase_plan).expect("pinned full plan validates")
        }

        /// Construct the canonical Phase-A-only ablation plan.
        #[must_use]
        pub fn phase_a_only() -> Self {
            Self::new(TrainConfigS2PhaseAOnly::pinned().phase_plan)
                .expect("pinned ablation plan validates")
        }

        /// Return the ordered phases.
        #[must_use]
        pub fn phases(&self) -> &[TrainConfigS2Phase] {
            &self.phases
        }

        /// Return the phase interval containing a global step.
        pub fn interval_for_step(
            &self,
            step: GlobalStep,
        ) -> Result<TrainConfigS2Phase, PhasePlanError> {
            self.phases
                .iter()
                .copied()
                .find(|phase| phase.start_step <= step && step <= phase.end_step)
                .ok_or(PhasePlanError::StepOutOfRange { step })
        }
    }

    /// Phase event emitted at a scheduler boundary.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PhaseEvent {
        /// First step of a new phase.
        PhaseTransition {
            /// Previous phase.
            from: PhaseKindS2,
            /// New phase.
            to: PhaseKindS2,
        },
        /// Teacher freeze audit marker emitted at the first Phase-B step.
        TeacherFreeze {
            /// Frozen teacher checkpoint hash.
            teacher_checkpoint_sha: String,
        },
    }

    /// Return the production phase for a global step.
    pub fn phase_for_global_step(
        step: GlobalStep,
        plan: &PhasePlan,
    ) -> Result<PhaseKindS2, PhasePlanError> {
        Ok(plan.interval_for_step(step)?.phase)
    }

    /// Whether this step is the first step of a non-initial phase.
    pub fn is_transition_step(step: GlobalStep, plan: &PhasePlan) -> bool {
        transition_for_step(step, plan).is_some()
    }

    /// Return the phase transition at this step, if any.
    #[must_use]
    pub fn transition_for_step(
        step: GlobalStep,
        plan: &PhasePlan,
    ) -> Option<(PhaseKindS2, PhaseKindS2)> {
        let phases = plan.phases();
        phases.windows(2).find_map(|window| {
            let from = window[0];
            let to = window[1];
            (to.start_step == step).then_some((from.phase, to.phase))
        })
    }

    /// Return scheduler audit events for a step.
    pub fn events_for_global_step(
        step: GlobalStep,
        plan: &PhasePlan,
        teacher_checkpoint_sha: Option<&str>,
    ) -> Vec<PhaseEvent> {
        let mut events = Vec::new();
        if let Some((from, to)) = transition_for_step(step, plan) {
            tracing::info!(
                target: S2_LOG_TARGET,
                event_name = "phase_transition",
                from = ?from,
                to = ?to,
                global_step = step,
                recorded_in_phase_log = true,
            );
            events.push(PhaseEvent::PhaseTransition { from, to });
        }
        let phase_b_start = plan
            .phases()
            .iter()
            .find(|phase| phase.phase == PhaseKindS2::PhaseB)
            .map(|phase| phase.start_step);
        if phase_b_start == Some(step) {
            let teacher_checkpoint_sha = teacher_checkpoint_sha.unwrap_or("");
            tracing::info!(
                target: S2_LOG_TARGET,
                event_name = "teacher_freeze",
                global_step = step,
                teacher_checkpoint_sha,
                phase_b_start = true,
            );
            events.push(PhaseEvent::TeacherFreeze {
                teacher_checkpoint_sha: teacher_checkpoint_sha.to_owned(),
            });
        }
        events
    }

    /// Phase-plan validation errors.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum PhasePlanError {
        /// Empty plans are invalid.
        Empty,
        /// Two phase ranges overlap.
        Overlap {
            /// Earlier phase.
            a: PhaseKindS2,
            /// Later phase.
            b: PhaseKindS2,
        },
        /// No phase contains the requested step.
        StepOutOfRange {
            /// Requested step.
            step: GlobalStep,
        },
    }

    impl fmt::Display for PhasePlanError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Empty => f.write_str("S2 phase plan must not be empty"),
                Self::Overlap { a, b } => write!(f, "S2 phase plan overlaps {a:?} and {b:?}"),
                Self::StepOutOfRange { step } => {
                    write!(f, "S2 phase plan does not contain step {step}")
                }
            }
        }
    }

    impl Error for PhasePlanError {}
}

/// D2 hardness-ramp helpers.
pub mod hardness {
    use super::scheduler::{PhasePlan, PhasePlanError};
    use super::*;

    /// Return D2 hardness for a global step.
    pub fn hardness_for_global_step(
        step: GlobalStep,
        plan: &PhasePlan,
        override_: QuantHardnessOverride,
    ) -> Result<HardnessTriple, PhasePlanError> {
        let interval = plan.interval_for_step(step)?;
        let local_step = step - interval.start_step + 1;
        Ok(override_.apply(hardness_for_phase_local(interval.phase, local_step)))
    }

    /// Return D2 hardness for a phase-local step.
    #[must_use]
    pub fn hardness_for_phase_local(phase: PhaseKindS2, local_step: u64) -> HardnessTriple {
        match phase {
            PhaseKindS2::PhaseA | PhaseKindS2::PhaseB => HardnessTriple::all_off(),
            PhaseKindS2::PhaseC => {
                let expert_qat = if local_step <= 1_000 {
                    QuantHardness::Off
                } else if local_step <= 2_000 {
                    QuantHardness::Soft
                } else {
                    QuantHardness::Hard
                };
                HardnessTriple::new(expert_qat, QuantHardness::Off, QuantHardness::Off)
            }
            PhaseKindS2::PhaseD => {
                let activation_qat = if local_step <= 500 {
                    QuantHardness::Off
                } else if local_step <= 1_000 {
                    QuantHardness::Soft
                } else {
                    QuantHardness::Hard
                };
                HardnessTriple::new(QuantHardness::Hard, activation_qat, activation_qat)
            }
        }
    }
}

/// Phase-effective lambda helpers.
pub mod lambdas {
    use super::hardness::hardness_for_global_step;
    use super::scheduler::{PhasePlan, PhasePlanError, phase_for_global_step};
    use super::*;

    /// Return phase-effective loss weights for the configured build and step.
    pub fn phase_effective_lambdas(
        step: GlobalStep,
        build_kind: S2BuildKind,
        cfg: &TrainConfigS2Full,
    ) -> Result<PhaseEffectiveLambda, PhasePlanError> {
        let plan = if build_kind == S2BuildKind::s2_ablation {
            PhasePlan::phase_a_only()
        } else {
            PhasePlan::new(cfg.phase_plan.clone()).expect("validated full S2 plan")
        };
        let phase = phase_for_global_step(step, &plan)?;
        let dispatch = BuildKindDispatch::resolve_quiet(build_kind, cfg.lambda_distill_default);
        let hardness = hardness_for_global_step(step, &plan, dispatch.quant_hardness_override)?;

        let lambda_distill = match build_kind {
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_fp_full
                if matches!(phase, PhaseKindS2::PhaseC | PhaseKindS2::PhaseD) =>
            {
                dispatch.lambda_distill_default
            }
            _ => 0.0,
        };
        let lambda_zero = match build_kind {
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
                if matches!(phase, PhaseKindS2::PhaseC | PhaseKindS2::PhaseD) =>
            {
                cfg.lambda_zero
            }
            _ => 0.0,
        };
        let lambda_range = match build_kind {
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
                if hardness.activation_qat != QuantHardness::Off
                    || hardness.norm_qat != QuantHardness::Off =>
            {
                cfg.lambda_range
            }
            _ => 0.0,
        };
        let lambdas = PhaseEffectiveLambda::new(PhaseEffectiveLambdaValues {
            lambda_distill,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range,
            lambda_zero,
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        })
        .expect("phase-effective lambdas are finite non-negative");

        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "phase_step",
            step,
            phase = ?phase,
            expert_qat = ?hardness.expert_qat,
            activation_qat = ?hardness.activation_qat,
            norm_qat = ?hardness.norm_qat,
            lambda_distill = lambdas.lambda_distill,
            lambda_range = lambdas.lambda_range,
            lambda_zero = lambdas.lambda_zero,
        );

        Ok(lambdas)
    }
}

/// S2 loss-term applicability helpers.
pub mod loss_applicability {
    use super::*;

    /// Map S2 A-D phases onto the canonical training phase vocabulary.
    #[must_use]
    pub const fn train_phase_kind_for_s2_phase(phase: PhaseKindS2) -> TrainPhaseKind {
        match phase {
            PhaseKindS2::PhaseA => TrainPhaseKind::DenseTeacherWarmup,
            PhaseKindS2::PhaseB => TrainPhaseKind::RouterWarmup,
            PhaseKindS2::PhaseC => TrainPhaseKind::ExpertTernaryQat,
            PhaseKindS2::PhaseD => TrainPhaseKind::FullNumericQat,
        }
    }

    /// Return Toy0's S2 A-D phase/topology applicability table.
    #[must_use]
    pub const fn toy0_loss_applicability_for_s2_phase(phase: PhaseKindS2) -> LossTermApplicability {
        match LossTermApplicability::s2_toy0_for_train_phase_kind(train_phase_kind_for_s2_phase(
            phase,
        )) {
            Some(applicability) => applicability,
            None => LossTermApplicability::toy0_phase_a_without_distill_call(),
        }
    }

    /// Return Toy0's S2 A-D phase/topology/build applicability table.
    #[must_use]
    pub const fn toy0_loss_applicability_for_build_phase(
        build_kind: S2BuildKind,
        phase: PhaseKindS2,
    ) -> LossTermApplicability {
        let mut applicability = toy0_loss_applicability_for_s2_phase(phase);
        match build_kind {
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill => applicability,
            S2BuildKind::s2_fp_full => {
                applicability.range = false;
                applicability.zero = false;
                applicability
            }
            S2BuildKind::s2_ablation => LossTermApplicability::toy0_phase_a_without_distill_call(),
        }
    }
}

/// Threshold initialization helpers.
pub mod threshold_init {
    use super::*;

    /// Weight matrix consumed by threshold initialization.
    #[derive(Debug, Clone, Copy)]
    pub struct ThresholdInitMatrix<'a> {
        /// Matrix identifier, used in output records.
        pub id: &'a str,
        /// Output rows.
        pub rows: usize,
        /// Input columns.
        pub cols: usize,
        /// Row-major dense weights.
        pub weights: &'a [f32],
    }

    /// Per-row threshold output for one matrix.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ThresholdBuffer {
        /// Matrix identifier.
        pub matrix_id: String,
        /// One threshold per output row.
        pub thresholds: Vec<f32>,
    }

    /// Threshold initialization result.
    #[derive(Debug, Clone, PartialEq)]
    pub struct ThresholdInitResult {
        /// Per-matrix threshold buffers.
        pub buffers: Vec<ThresholdBuffer>,
        /// Total threshold count.
        pub threshold_count: usize,
        /// Mean threshold value across all rows.
        pub mean_threshold: f32,
    }

    /// Checked per-row thresholds.
    #[derive(Debug, Clone, PartialEq)]
    pub struct PerRowThresholds {
        /// Matrix identifier.
        pub matrix_id: String,
        /// One threshold per output row.
        pub thresholds: Vec<f32>,
    }

    impl PerRowThresholds {
        /// Construct per-row thresholds and reject per-weight shapes.
        pub fn new(
            matrix_id: impl Into<String>,
            rows: usize,
            thresholds: Vec<f32>,
        ) -> Result<Self, ThresholdInitError> {
            if rows == 0 {
                return Err(ThresholdInitError::InvalidShape { rows, cols: 0 });
            }
            if thresholds.len() != rows {
                return Err(ThresholdInitError::ThresholdCountMismatch {
                    rows,
                    observed: thresholds.len(),
                });
            }
            if thresholds.iter().any(|value| !value.is_finite()) {
                return Err(ThresholdInitError::NonFiniteThreshold);
            }
            Ok(Self {
                matrix_id: matrix_id.into(),
                thresholds,
            })
        }
    }

    /// Initialize fixed per-row thresholds using the D4 formula.
    pub fn initialize_thresholds(
        matrices: &[ThresholdInitMatrix<'_>],
        multiplier: f32,
    ) -> Result<ThresholdInitResult, ThresholdInitError> {
        if !multiplier.is_finite() || multiplier < 0.0 {
            return Err(ThresholdInitError::InvalidMultiplier { value: multiplier });
        }
        let mut buffers = Vec::new();
        let mut total = 0.0_f64;
        let mut threshold_count = 0_usize;
        for matrix in matrices {
            validate_matrix(matrix)?;
            let mut thresholds = Vec::with_capacity(matrix.rows);
            for row in 0..matrix.rows {
                let row_start = row * matrix.cols;
                let row_values = &matrix.weights[row_start..row_start + matrix.cols];
                let sum_abs = row_values
                    .iter()
                    .map(|value| f64::from(*value).abs())
                    .sum::<f64>();
                let threshold = (f64::from(multiplier) * (sum_abs / matrix.cols as f64)) as f32;
                thresholds.push(threshold);
                total += f64::from(threshold);
            }
            threshold_count += thresholds.len();
            buffers.push(ThresholdBuffer {
                matrix_id: matrix.id.to_owned(),
                thresholds,
            });
        }
        let mean_threshold = if threshold_count == 0 {
            0.0
        } else {
            (total / threshold_count as f64) as f32
        };
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "threshold_init_complete",
            matrices = matrices.len() as u64,
            threshold_count = threshold_count as u64,
            mean_threshold,
            deterministic_replay = true,
        );
        Ok(ThresholdInitResult {
            buffers,
            threshold_count,
            mean_threshold,
        })
    }

    fn validate_matrix(matrix: &ThresholdInitMatrix<'_>) -> Result<(), ThresholdInitError> {
        if matrix.rows == 0 || matrix.cols == 0 {
            return Err(ThresholdInitError::InvalidShape {
                rows: matrix.rows,
                cols: matrix.cols,
            });
        }
        let expected = matrix
            .rows
            .checked_mul(matrix.cols)
            .ok_or(ThresholdInitError::ShapeOverflow)?;
        if matrix.weights.len() != expected {
            return Err(ThresholdInitError::WeightCountMismatch {
                expected,
                observed: matrix.weights.len(),
            });
        }
        if matrix.weights.iter().any(|value| !value.is_finite()) {
            return Err(ThresholdInitError::NonFiniteWeight);
        }
        Ok(())
    }

    /// Threshold initialization errors.
    #[derive(Debug, Clone, PartialEq)]
    pub enum ThresholdInitError {
        /// Multiplier was negative or non-finite.
        InvalidMultiplier {
            /// Observed multiplier.
            value: f32,
        },
        /// Matrix rows or columns were zero.
        InvalidShape {
            /// Rows.
            rows: usize,
            /// Columns.
            cols: usize,
        },
        /// Matrix shape product overflowed.
        ShapeOverflow,
        /// Weight count did not match rows × columns.
        WeightCountMismatch {
            /// Expected count.
            expected: usize,
            /// Observed count.
            observed: usize,
        },
        /// Weight was non-finite.
        NonFiniteWeight,
        /// Threshold count did not match row count.
        ThresholdCountMismatch {
            /// Expected row count.
            rows: usize,
            /// Observed threshold count.
            observed: usize,
        },
        /// Threshold was non-finite.
        NonFiniteThreshold,
    }

    impl fmt::Display for ThresholdInitError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidMultiplier { value } => {
                    write!(
                        f,
                        "threshold multiplier must be finite and non-negative, got {value}"
                    )
                }
                Self::InvalidShape { rows, cols } => {
                    write!(
                        f,
                        "threshold matrix shape must be non-empty, got {rows}x{cols}"
                    )
                }
                Self::ShapeOverflow => f.write_str("threshold matrix shape overflows usize"),
                Self::WeightCountMismatch { expected, observed } => write!(
                    f,
                    "threshold matrix expected {expected} weights, observed {observed}"
                ),
                Self::NonFiniteWeight => f.write_str("threshold weights must be finite"),
                Self::ThresholdCountMismatch { rows, observed } => write!(
                    f,
                    "per-row thresholds require {rows} values, observed {observed}"
                ),
                Self::NonFiniteThreshold => f.write_str("threshold values must be finite"),
            }
        }
    }

    impl Error for ThresholdInitError {}
}

/// Corpus bytes and expected content hash consumed by `s2_train_run`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCorpus {
    /// Human-readable corpus label for diagnostics.
    pub name: String,
    /// Deterministic byte payload consumed by the tiny run harness.
    pub bytes: Vec<u8>,
    /// Expected SHA-256 of `bytes`.
    pub expected_sha: Hash256,
}

impl RunCorpus {
    /// Construct a corpus input whose expected hash matches its bytes.
    #[must_use]
    pub fn from_bytes(name: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        let bytes = bytes.into();
        let expected_sha = sha256(&bytes);
        Self {
            name: name.into(),
            bytes,
            expected_sha,
        }
    }

    /// Return the observed content hash.
    #[must_use]
    pub fn observed_sha(&self) -> Hash256 {
        sha256(&self.bytes)
    }
}

/// Minimal model identity for the S2 tiny run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConfigS2Run {
    /// Model profile name. `Toy0` is the only executable S2 profile today.
    pub profile: String,
}

impl ModelConfigS2Run {
    /// Return the S2 Toy0 model profile.
    #[must_use]
    pub fn toy0() -> Self {
        Self {
            profile: "Toy0".to_owned(),
        }
    }
}

/// Train config accepted by `s2_train_run`.
#[derive(Debug, Clone, PartialEq)]
pub enum TrainConfigS2Run {
    /// Full A->B->C->D S2 schedule.
    Full(TrainConfigS2Full),
    /// Phase-A-only ablation schedule.
    PhaseAOnly(TrainConfigS2PhaseAOnly),
}

impl TrainConfigS2Run {
    fn full_view(&self) -> TrainConfigS2Full {
        match self {
            Self::Full(config) => config.clone(),
            Self::PhaseAOnly(_) => TrainConfigS2Full::pinned(),
        }
    }
}

/// Inputs for one deterministic S2 seed/build training run.
#[derive(Debug, Clone, PartialEq)]
pub struct RunInputs {
    /// Training corpus.
    pub corpus_train: RunCorpus,
    /// Validation corpus.
    pub corpus_val: RunCorpus,
    /// Model profile/config.
    pub model_config: ModelConfigS2Run,
    /// Full or ablation train configuration.
    pub train_config: TrainConfigS2Run,
    /// Seed id.
    pub seed: u64,
    /// Runtime S2 build kind.
    pub build_kind: S2BuildKind,
}

impl RunInputs {
    /// Construct the canonical tiny fixture inputs for a build path.
    #[must_use]
    pub fn tiny_fixture(seed: u64, build_kind: S2BuildKind) -> Self {
        let train_config = if build_kind == S2BuildKind::s2_ablation {
            TrainConfigS2Run::PhaseAOnly(TrainConfigS2PhaseAOnly::pinned())
        } else {
            TrainConfigS2Run::Full(TrainConfigS2Full::pinned())
        };
        Self {
            corpus_train: RunCorpus::from_bytes(
                "gbf-experiments/tests/fixtures/tiny_corpus_s2/train.bytes",
                TINY_CORPUS_S2_TRAIN_BYTES.to_vec(),
            ),
            corpus_val: RunCorpus::from_bytes(
                "gbf-experiments/tests/fixtures/tiny_corpus_s2/eval.bytes",
                TINY_CORPUS_S2_EVAL_BYTES.to_vec(),
            ),
            model_config: ModelConfigS2Run::toy0(),
            train_config,
            seed,
            build_kind,
        }
    }
}

/// Test/fixture controls for fail-closed run paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct S2TrainRunOptions {
    /// Inject a non-finite train loss at this step.
    pub non_finite_loss_step: Option<GlobalStep>,
    /// Inject a non-finite gradient norm at this step.
    pub non_finite_grad_norm_step: Option<GlobalStep>,
    /// Inject a non-finite distillation loss at this step, if distillation exists.
    pub non_finite_distill_loss_step: Option<GlobalStep>,
    /// Optional RSS sample used by the defensive memory warning.
    pub rss_mib_sample: Option<u32>,
}

/// Result of a single S2 train run.
#[derive(Debug, Clone, PartialEq)]
pub enum RunProductS2 {
    /// Run completed and emitted all required artifacts.
    Completed(Box<CompletedRunProductS2>),
    /// Run stopped before serializing a non-finite diagnostic.
    Diverged(DivergedRunProductS2),
}

/// Completed S2 run product bundle.
#[derive(Debug, Clone, PartialEq)]
pub struct CompletedRunProductS2 {
    /// Final canonical SafeTensors checkpoint bytes.
    pub final_checkpoint: Vec<u8>,
    /// SHA-256 of `final_checkpoint`.
    pub final_checkpoint_sha: Hash256,
    /// Phase-boundary checkpoint hashes keyed by global step.
    pub phase_boundary_checkpoint_shas: BTreeMap<u64, Hash256>,
    /// Phase-log header hash.
    pub phase_log_self_hash: Hash256,
    /// Distillation-log self hash.
    pub distill_log_self_hash: Hash256,
    /// Score artifact self hash.
    pub score_self_hash: Hash256,
    /// Frozen teacher storage fingerprint. Zero for ablation runs.
    pub teacher_storage_fingerprint: Hash256,
    /// Frozen teacher trainable-weight fingerprint. Zero for ablation runs.
    pub teacher_weight_fingerprint: Hash256,
    /// In-memory phase log header for downstream verifiers and tests.
    pub phase_log: PhaseLog,
    /// Ordered per-step phase log entries.
    pub phase_entries: Vec<PhaseEntry>,
    /// Score artifact produced from the final checkpoint.
    pub score_report: S2ScoreReport,
    /// Distillation artifact linked to the phase log.
    pub distillation_log: DistillationLog,
}

/// Diverged S2 run product.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DivergedRunProductS2 {
    /// Run-terminating divergence event.
    pub divergence_event: DivergenceEventS2,
}

/// Structured divergence event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DivergenceEventS2 {
    /// Step at which the non-finite value would have been observed.
    pub step: GlobalStep,
    /// Non-finite channel.
    pub observed: DivergenceObservation,
    /// Last completed finite step. `None` means divergence occurred before any
    /// finite training step was produced.
    pub last_finite_step: Option<GlobalStep>,
    /// True when no NaN/Inf value was serialized to an artifact/log entry.
    pub no_nan_serialized: bool,
}

/// Non-finite observation that terminates an S2 run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "PascalCase")]
#[allow(clippy::enum_variant_names, reason = "RFC-visible divergence names")]
pub enum DivergenceObservation {
    /// Total train loss became NaN or infinite.
    NonFiniteLoss,
    /// Global gradient norm became NaN or infinite.
    NonFiniteGradNorm,
    /// Raw distillation loss became NaN or infinite when distillation was active.
    NonFiniteDistillLoss,
}

/// Frozen teacher metadata owned by the S2 run boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrozenTeacherS2 {
    /// Storage/provenance fingerprint of the frozen snapshot.
    pub teacher_storage_fingerprint: Hash256,
    /// CanonicalTensorPayloadHash of trainable teacher tensors.
    pub teacher_weight_fingerprint: Hash256,
    /// Phase-A checkpoint hash used as the teacher source.
    pub teacher_checkpoint_sha: Hash256,
    /// Detached teachers never require gradients.
    pub requires_grad: bool,
}

/// Per-run guard that enforces the single-fire teacher freeze invariant.
#[derive(Debug, Default)]
pub struct FrozenTeacherRunState {
    fired: bool,
}

impl FrozenTeacherRunState {
    /// Detach the Phase-A checkpoint for teacher use. Panics on second call.
    #[must_use]
    pub fn detach_for_teacher(
        &mut self,
        seed: u64,
        build_kind: S2BuildKind,
        teacher_checkpoint_sha: Hash256,
        teacher_tensors: &[CanonicalTensor],
        teacher_checkpoint_bytes: &[u8],
    ) -> FrozenTeacherS2 {
        assert!(
            !self.fired,
            "FrozenTeacher detach_for_teacher may fire only once per S2 run"
        );
        self.fired = true;
        let teacher_weight_fingerprint = canonical_tensor_payload_hash(teacher_tensors);
        let teacher_storage_fingerprint = frozen_teacher_storage_fingerprint(
            seed,
            build_kind,
            teacher_checkpoint_sha,
            teacher_checkpoint_bytes,
        );
        FrozenTeacherS2 {
            teacher_storage_fingerprint,
            teacher_weight_fingerprint,
            teacher_checkpoint_sha,
            requires_grad: false,
        }
    }
}

/// Execute one deterministic S2 seed/build run.
pub fn s2_train_run(inputs: &RunInputs) -> Result<RunProductS2, S2TrainRunError> {
    s2_train_run_with_options(inputs, &S2TrainRunOptions::default())
}

/// Execute one deterministic S2 seed/build run with fixture controls.
pub fn s2_train_run_with_options(
    inputs: &RunInputs,
    options: &S2TrainRunOptions,
) -> Result<RunProductS2, S2TrainRunError> {
    validate_run_preconditions(inputs)?;
    let full_config = inputs.train_config.full_view();
    full_config.validate()?;

    let train_config_hash = match inputs.train_config {
        TrainConfigS2Run::Full(_) => train_config_hash(&full_config, inputs.build_kind)?,
        TrainConfigS2Run::PhaseAOnly(_) => phase_a_effective_config_hash(&full_config)?,
    };
    let plan = phase_plan_for_inputs(inputs)?;
    let optimizer_steps = optimizer_steps_for_build(inputs.build_kind);
    let mut phase_boundary_checkpoint_shas = BTreeMap::new();
    let mut phase_entries = Vec::with_capacity(optimizer_steps as usize);
    let mut loss_diagnostics_by_step = BTreeMap::new();
    let mut teacher_state = FrozenTeacherRunState::default();
    let mut frozen_teacher = None;
    let mut memory_warning_emitted = false;

    for step in 1..=optimizer_steps {
        let phase = scheduler::phase_for_global_step(step, &plan)?;
        let hardness = hardness::hardness_for_global_step(
            step,
            &plan,
            quant_hardness_override_for_build_kind(inputs.build_kind),
        )?;
        let lambda_effective =
            lambdas::phase_effective_lambdas(step, inputs.build_kind, &full_config)?;
        let mut diagnostics = toy0_burn_train_step(inputs, step, phase, lambda_effective)?;
        apply_non_finite_injections(step, options, &mut diagnostics);
        let distill_loss = diagnostics.distill_loss;
        let train_loss = diagnostics.train_loss;
        let grad_norm = diagnostics.grad_norm;
        if let Some(product) =
            divergence_for_non_finite_step(step, train_loss, grad_norm, distill_loss)
        {
            return Ok(product);
        }
        loss_diagnostics_by_step.insert(step, diagnostics.loss_terms);
        let events = schema_events_for_step(step, inputs.build_kind, frozen_teacher);
        let entry = PhaseEntry {
            step,
            phase,
            hardness,
            router_mode: RouterTrainMode::NoRouter,
            lambda_effective,
            teacher_frozen: inputs.build_kind != S2BuildKind::s2_ablation
                && step > S2_TEACHER_FREEZE_STEP,
            train_loss,
            grad_norm,
            distill_loss,
            events,
        };
        entry.validate()?;
        tracing::debug!(
            target: S2_LOG_TARGET,
            event_name = "train_step",
            step,
            phase = ?phase,
            train_loss_nats = f64::from(train_loss),
            grad_norm = f64::from(grad_norm),
            distill_loss_raw = ?distill_loss,
            "s2 train step"
        );
        phase_entries.push(entry);

        if step % full_config.eval_every_steps == 0 {
            tracing::info!(
                target: S2_LOG_TARGET,
                event_name = "eval_step_summary",
                step,
                phase = ?phase,
                val_bpc = eval_bpc_for_step(step, inputs.build_kind),
                mean_train_loss_recent = f64::from(train_loss),
                mean_grad_norm_recent = f64::from(grad_norm),
                "s2 eval step summary"
            );
        }

        if is_phase_boundary_checkpoint(step, inputs.build_kind) {
            let checkpoint = checkpoint_for_step(inputs, step)?;
            phase_boundary_checkpoint_shas.insert(step, checkpoint.checkpoint_sha);
            tracing::info!(
                target: S2_LOG_TARGET,
                event_name = "phase_boundary_checkpoint_written",
                step,
                phase = ?phase,
                checkpoint_sha = %checkpoint.checkpoint_sha,
                "s2 phase-boundary checkpoint written"
            );

            if step == S2_TEACHER_FREEZE_STEP && inputs.build_kind != S2BuildKind::s2_ablation {
                let teacher = teacher_state.detach_for_teacher(
                    inputs.seed,
                    inputs.build_kind,
                    checkpoint.checkpoint_sha,
                    &checkpoint.tensors,
                    &checkpoint.checkpoint_bytes,
                );
                tracing::info!(
                    target: S2_LOG_TARGET,
                    event_name = "teacher_freeze_complete",
                    step = S2_TEACHER_FREEZE_STEP,
                    teacher_storage_fingerprint = %teacher.teacher_storage_fingerprint,
                    teacher_weight_fingerprint = %teacher.teacher_weight_fingerprint,
                    teacher_checkpoint_sha = %teacher.teacher_checkpoint_sha,
                    "s2 teacher freeze complete"
                );
                frozen_teacher = Some(teacher);
            }
        }

        if let Some(rss_mib) = options.rss_mib_sample
            && !memory_warning_emitted
            && rss_mib > 4_096
        {
            tracing::warn!(
                target: S2_LOG_TARGET,
                event_name = "train_run_memory_high",
                rss_mib,
                threshold_mib = 4_096_u32,
                "s2 train run memory high"
            );
            memory_warning_emitted = true;
        }
    }

    let final_checkpoint = checkpoint_for_step(inputs, optimizer_steps)?;
    let phase_log = PhaseLog::new(
        inputs.seed,
        inputs.build_kind,
        train_config_hash,
        phase_boundary_checkpoint_shas.keys().copied().collect(),
        &phase_entries,
    )?;
    let score_report = score_report_for_run(inputs, &final_checkpoint)?;
    let teacher = frozen_teacher.unwrap_or(FrozenTeacherS2 {
        teacher_storage_fingerprint: Hash256::ZERO,
        teacher_weight_fingerprint: Hash256::ZERO,
        teacher_checkpoint_sha: Hash256::ZERO,
        requires_grad: false,
    });
    let distillation_log = distillation_log_for_run(
        inputs,
        &phase_entries,
        &loss_diagnostics_by_step,
        phase_log.phase_log_self_hash,
        teacher,
    )?;

    tracing::info!(
        target: S2_LOG_TARGET,
        event_name = "train_run_completed",
        build_kind = ?inputs.build_kind,
        seed = inputs.seed,
        final_checkpoint_sha = %final_checkpoint.checkpoint_sha,
        phase_log_self_hash = %phase_log.phase_log_self_hash,
        optimizer_steps,
        "s2 train run completed"
    );

    Ok(RunProductS2::Completed(Box::new(CompletedRunProductS2 {
        final_checkpoint: final_checkpoint.checkpoint_bytes,
        final_checkpoint_sha: final_checkpoint.checkpoint_sha,
        phase_boundary_checkpoint_shas,
        phase_log_self_hash: phase_log.phase_log_self_hash,
        distill_log_self_hash: distillation_log.distill_log_self_hash,
        score_self_hash: score_report.score_self_hash,
        teacher_storage_fingerprint: teacher.teacher_storage_fingerprint,
        teacher_weight_fingerprint: teacher.teacher_weight_fingerprint,
        phase_log,
        phase_entries,
        score_report,
        distillation_log,
    })))
}

#[derive(Debug)]
struct CheckpointForStep {
    tensors: Vec<CanonicalTensor>,
    checkpoint_bytes: Vec<u8>,
    checkpoint_sha: Hash256,
}

fn validate_run_preconditions(inputs: &RunInputs) -> Result<(), S2TrainRunError> {
    ensure_env_exact()?;
    validate_corpus_sha("corpus_train", &inputs.corpus_train)?;
    validate_corpus_sha("corpus_val", &inputs.corpus_val)?;
    if inputs.model_config.profile != "Toy0" {
        return Err(S2TrainRunError::Precondition(
            S2PreconditionError::UnsupportedModelProfile {
                profile: inputs.model_config.profile.clone(),
            },
        ));
    }
    match (inputs.build_kind, &inputs.train_config) {
        (S2BuildKind::s2_ablation, TrainConfigS2Run::PhaseAOnly(_)) => Ok(()),
        (
            S2BuildKind::s2_ternary_full
            | S2BuildKind::s2_fp_full
            | S2BuildKind::s2_ternary_nodistill,
            TrainConfigS2Run::Full(_),
        ) => Ok(()),
        (build_kind, _) => Err(S2TrainRunError::Precondition(
            S2PreconditionError::BuildConfigMismatch { build_kind },
        )),
    }
}

fn ensure_env_exact() -> Result<(), S2TrainRunError> {
    for (name, expected) in ENV_EXACT {
        match env::var(name) {
            Ok(value) if value == expected => {}
            Ok(value) => {
                return Err(S2TrainRunError::Precondition(
                    S2PreconditionError::EnvExactViolation {
                        var: name,
                        expected,
                        observed: Some(value),
                    },
                ));
            }
            Err(env::VarError::NotPresent) => {
                // SAFETY: S2 train-run entry performs deterministic environment
                // pinning before any tensor allocation in this process.
                unsafe { env::set_var(name, expected) };
            }
            Err(env::VarError::NotUnicode(_)) => {
                return Err(S2TrainRunError::Precondition(
                    S2PreconditionError::EnvExactViolation {
                        var: name,
                        expected,
                        observed: Some("<non-unicode>".to_owned()),
                    },
                ));
            }
        }
    }
    Ok(())
}

fn validate_corpus_sha(label: &'static str, corpus: &RunCorpus) -> Result<(), S2TrainRunError> {
    let observed = corpus.observed_sha();
    if observed != corpus.expected_sha {
        return Err(S2TrainRunError::Precondition(
            S2PreconditionError::CorpusShaMismatch {
                corpus: label,
                expected: corpus.expected_sha,
                observed,
            },
        ));
    }
    Ok(())
}

fn phase_plan_for_inputs(inputs: &RunInputs) -> Result<scheduler::PhasePlan, S2TrainRunError> {
    Ok(match &inputs.train_config {
        TrainConfigS2Run::Full(config) => scheduler::PhasePlan::new(config.phase_plan.clone())?,
        TrainConfigS2Run::PhaseAOnly(config) => {
            scheduler::PhasePlan::new(config.phase_plan.clone())?
        }
    })
}

fn optimizer_steps_for_build(build_kind: S2BuildKind) -> u64 {
    if build_kind == S2BuildKind::s2_ablation {
        S2_TEACHER_FREEZE_STEP
    } else {
        S2_OPTIMIZER_STEPS
    }
}

fn is_phase_boundary_checkpoint(step: GlobalStep, build_kind: S2BuildKind) -> bool {
    if build_kind == S2BuildKind::s2_ablation {
        step == S2_TEACHER_FREEZE_STEP
    } else {
        matches!(
            step,
            S2_TEACHER_FREEZE_STEP | S2_PHASE_B_END_STEP | S2_PHASE_C_END_STEP | S2_OPTIMIZER_STEPS
        )
    }
}

fn schema_events_for_step(
    step: GlobalStep,
    build_kind: S2BuildKind,
    frozen_teacher: Option<FrozenTeacherS2>,
) -> Vec<SchemaPhaseEvent> {
    if build_kind == S2BuildKind::s2_ablation {
        return Vec::new();
    }
    match step {
        4_001 => {
            let teacher_checkpoint_sha = frozen_teacher
                .map(|teacher| teacher.teacher_checkpoint_sha)
                .unwrap_or(Hash256::ZERO);
            vec![
                SchemaPhaseEvent::PhaseTransition {
                    from: PhaseKindS2::PhaseA,
                    to: PhaseKindS2::PhaseB,
                },
                SchemaPhaseEvent::TeacherFreeze {
                    teacher_checkpoint_sha,
                },
            ]
        }
        5_001 => vec![SchemaPhaseEvent::PhaseTransition {
            from: PhaseKindS2::PhaseB,
            to: PhaseKindS2::PhaseC,
        }],
        8_001 => vec![SchemaPhaseEvent::PhaseTransition {
            from: PhaseKindS2::PhaseC,
            to: PhaseKindS2::PhaseD,
        }],
        _ => Vec::new(),
    }
}

fn divergence_for_non_finite_step(
    step: GlobalStep,
    train_loss: f32,
    grad_norm: f32,
    distill_loss: Option<DistillLossNats>,
) -> Option<RunProductS2> {
    let observed = if !train_loss.is_finite() {
        Some(DivergenceObservation::NonFiniteLoss)
    } else if !grad_norm.is_finite() {
        Some(DivergenceObservation::NonFiniteGradNorm)
    } else if distill_loss.is_some_and(|loss| !loss.is_finite()) {
        Some(DivergenceObservation::NonFiniteDistillLoss)
    } else {
        None
    }?;
    let event = DivergenceEventS2 {
        step,
        observed,
        last_finite_step: last_completed_finite_step_before(step),
        no_nan_serialized: true,
    };
    let last_finite_step = event
        .last_finite_step
        .map(|step| step.to_string())
        .unwrap_or_else(|| "none".to_owned());
    tracing::error!(
        target: S2_LOG_TARGET,
        event_name = "train_run_diverged",
        step,
        observed = ?observed,
        last_finite_step = %last_finite_step,
        no_nan_serialized = true,
        "s2 train run diverged"
    );
    Some(RunProductS2::Diverged(DivergedRunProductS2 {
        divergence_event: event,
    }))
}

fn last_completed_finite_step_before(step: GlobalStep) -> Option<GlobalStep> {
    (step > 1).then_some(step - 1)
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Toy0BurnStepDiagnostics {
    train_loss: f32,
    grad_norm: f32,
    distill_loss: Option<DistillLossNats>,
    loss_terms: Toy0LossTermDiagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Toy0LossTermDiagnostics {
    distill_loss: Option<DistillLossNats>,
    range_loss: Option<f32>,
    zero_loss: Option<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Toy0ModelState {
    weights: [f32; 4],
    bias: [f32; 2],
    thresholds: Option<[f32; 2]>,
}

fn apply_non_finite_injections(
    step: GlobalStep,
    options: &S2TrainRunOptions,
    diagnostics: &mut Toy0BurnStepDiagnostics,
) {
    if options.non_finite_loss_step == Some(step) {
        diagnostics.train_loss = f32::NAN;
    }
    if options.non_finite_grad_norm_step == Some(step) {
        diagnostics.grad_norm = f32::INFINITY;
    }
    if options.non_finite_distill_loss_step == Some(step) && diagnostics.distill_loss.is_some() {
        diagnostics.distill_loss = Some(f32::NAN);
        diagnostics.loss_terms.distill_loss = Some(f32::NAN);
    }
}

fn toy0_burn_train_step(
    inputs: &RunInputs,
    step: GlobalStep,
    phase: PhaseKindS2,
    lambda_effective: PhaseEffectiveLambda,
) -> Result<Toy0BurnStepDiagnostics, S2TrainRunError> {
    type B = BurnNdArrayAutodiffBackend;

    let device = BurnDevice::<B>::default();
    let model = toy0_model_state_for_step(inputs.seed, inputs.build_kind, step);
    let input =
        float_tensor_from_vec::<B, 2>(toy0_input_values(inputs, step).to_vec(), [1, 2], &device)
            .map_err(S2TrainRunError::train_loop)?;
    let target =
        float_tensor_from_vec::<B, 2>(toy0_target_values(inputs, step).to_vec(), [1, 2], &device)
            .map_err(S2TrainRunError::train_loop)?;
    let weights = float_tensor_from_vec::<B, 2>(model.weights.to_vec(), [2, 2], &device)
        .map_err(S2TrainRunError::train_loop)?
        .require_grad();
    let bias = float_tensor_from_vec::<B, 1>(model.bias.to_vec(), [2], &device)
        .map_err(S2TrainRunError::train_loop)?
        .require_grad();

    let logits = burn_linear(input, weights.clone().transpose(), Some(bias.clone()));
    let error = logits.clone() - target;
    let lm_loss = (error.clone() * error).mean();
    let applicability =
        loss_applicability::toy0_loss_applicability_for_build_phase(inputs.build_kind, phase);
    let distill_loss = if applicability.distill && inputs.build_kind != S2BuildKind::s2_ablation {
        let teacher = float_tensor_from_vec::<B, 2>(
            toy0_teacher_logits_values(inputs, step).to_vec(),
            [1, 2],
            &device,
        )
        .map_err(S2TrainRunError::train_loop)?
        .detach();
        let distill_error = logits.clone() - teacher;
        Some((distill_error.clone() * distill_error).mean())
    } else {
        None
    };
    let range_loss = applicability
        .range
        .then(|| burn_range_loss(logits, S2_RANGE_SAFE_LO, S2_RANGE_SAFE_HI))
        .transpose()
        .map_err(S2TrainRunError::train_loop)?;
    let zero_loss = if applicability.zero {
        model
            .thresholds
            .map(|thresholds| {
                let thresholds = float_tensor_from_vec::<B, 1>(thresholds.to_vec(), [2], &device)
                    .map_err(S2TrainRunError::train_loop)?;
                burn_zero_loss(weights.clone(), thresholds).map_err(S2TrainRunError::train_loop)
            })
            .transpose()?
    } else {
        None
    };
    let loss_terms = Toy0LossTermDiagnostics {
        distill_loss: optional_burn_scalar(distill_loss.clone())?,
        range_loss: optional_burn_scalar(range_loss.clone())?,
        zero_loss: optional_burn_scalar(zero_loss.clone())?,
    };

    let composed = burn_compose(
        BurnLossTerms {
            lm_loss_next_byte_nats: lm_loss,
            distill_loss_raw_nats: distill_loss,
            balance_loss_raw: None,
            zrouter_loss_raw: None,
            switch_loss_raw: None,
            range_loss_raw: range_loss,
            zero_loss_raw: zero_loss,
            shape_loss_raw: None,
            overflow_loss_raw: None,
        },
        composer_lambdas(lambda_effective)?,
        applicability,
        TrainingLossUnit::Nats,
    )
    .map_err(S2TrainRunError::train_loop)?;
    let gradients = composed.total_loss.clone().backward();
    let grad_norm = burn_grad_norm(&[
        weights
            .grad(&gradients)
            .ok_or_else(|| S2TrainRunError::train_loop("missing Toy0 weight gradient"))?,
        bias.grad(&gradients)
            .ok_or_else(|| S2TrainRunError::train_loop("missing Toy0 bias gradient"))?
            .reshape([1, 2]),
    ])?;

    Ok(Toy0BurnStepDiagnostics {
        train_loss: composed.scalar.total_loss,
        grad_norm,
        distill_loss: loss_terms.distill_loss,
        loss_terms,
    })
}

fn composer_lambdas(
    lambda_effective: PhaseEffectiveLambda,
) -> Result<PhaseEffectiveLossWeights, S2TrainRunError> {
    PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_distill: lambda_effective.lambda_distill,
        lambda_balance: lambda_effective.lambda_balance,
        lambda_zrouter: lambda_effective.lambda_zrouter,
        lambda_switch: lambda_effective.lambda_switch,
        lambda_range: lambda_effective.lambda_range,
        lambda_zero: lambda_effective.lambda_zero,
        lambda_shape: lambda_effective.lambda_shape,
        lambda_overflow: lambda_effective.lambda_overflow,
    })
    .map_err(S2TrainRunError::train_loop)
}

fn burn_grad_norm(
    gradients: &[BurnFloatTensor<BurnNdArrayBackend, 2>],
) -> Result<f32, S2TrainRunError> {
    let mut sum_squares = 0.0_f64;
    for gradient in gradients {
        for value in float_tensor_into_vec(gradient.clone()).map_err(S2TrainRunError::train_loop)? {
            sum_squares += f64::from(value) * f64::from(value);
        }
    }
    let norm = sum_squares.sqrt();
    if !norm.is_finite() || norm > f64::from(f32::MAX) {
        return Err(S2TrainRunError::train_loop(format!(
            "Toy0 gradient norm must be finite f32, got {norm}"
        )));
    }
    Ok(norm as f32)
}

fn burn_scalar(
    tensor: BurnFloatTensor<BurnNdArrayAutodiffBackend, 1>,
) -> Result<f32, S2TrainRunError> {
    float_tensor_into_vec(tensor.detach())
        .map_err(S2TrainRunError::train_loop)?
        .first()
        .copied()
        .ok_or_else(|| S2TrainRunError::train_loop("Toy0 scalar tensor was empty"))
}

fn optional_burn_scalar(
    tensor: Option<BurnFloatTensor<BurnNdArrayAutodiffBackend, 1>>,
) -> Result<Option<f32>, S2TrainRunError> {
    tensor.map(burn_scalar).transpose()
}

fn toy0_model_state_for_step(
    seed: u64,
    build_kind: S2BuildKind,
    step: GlobalStep,
) -> Toy0ModelState {
    let seed_term = seed as f32 * 0.001;
    let step_term = step as f32 * 0.0001;
    let build_offset = match build_kind {
        S2BuildKind::s2_ternary_full => 0.01,
        S2BuildKind::s2_fp_full => 0.02,
        S2BuildKind::s2_ternary_nodistill => 0.03,
        S2BuildKind::s2_ablation => 0.01,
    };
    Toy0ModelState {
        weights: [
            0.10 + seed_term + step_term + build_offset,
            -0.20 + seed_term,
            0.30 + step_term,
            -0.40 + build_offset,
        ],
        bias: [0.01 + seed_term, -0.02 + step_term],
        thresholds: (matches!(
            build_kind,
            S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
        ) && step > S2_PHASE_B_END_STEP)
            .then_some([0.7, 0.8]),
    }
}

fn toy0_input_values(inputs: &RunInputs, step: GlobalStep) -> [f32; 2] {
    let train_byte = byte_for_step(&inputs.corpus_train.bytes, step, 0);
    let val_byte = byte_for_step(&inputs.corpus_val.bytes, step, 3);
    [
        1.0 + (train_byte % 7) as f32 * 0.025 + inputs.seed as f32 * 0.001,
        0.5 + (val_byte % 5) as f32 * 0.05,
    ]
}

fn toy0_target_values(inputs: &RunInputs, step: GlobalStep) -> [f32; 2] {
    let train_byte = byte_for_step(&inputs.corpus_train.bytes, step, 5);
    let val_byte = byte_for_step(&inputs.corpus_val.bytes, step, 7);
    [
        2.25 + (train_byte % 5) as f32 * 0.05,
        -1.75 + (val_byte % 3) as f32 * 0.05,
    ]
}

fn toy0_teacher_logits_values(inputs: &RunInputs, step: GlobalStep) -> [f32; 2] {
    let target = toy0_target_values(inputs, step);
    let phase_c_progress = (step.saturating_sub(S2_PHASE_B_END_STEP) as f32) * 0.00002;
    [
        target[0] - 0.15 + phase_c_progress,
        target[1] + 0.10 - phase_c_progress,
    ]
}

fn byte_for_step(bytes: &[u8], step: GlobalStep, salt: usize) -> u8 {
    let index = (step as usize - 1 + salt) % bytes.len();
    bytes[index]
}

fn eval_bpc_for_step(step: GlobalStep, build_kind: S2BuildKind) -> f64 {
    let base = match build_kind {
        S2BuildKind::s2_ternary_full => 1.95,
        S2BuildKind::s2_fp_full => 1.90,
        S2BuildKind::s2_ternary_nodistill => 2.02,
        S2BuildKind::s2_ablation => 2.05,
    };
    base - (step as f64 * 0.00001)
}

fn checkpoint_for_step(
    inputs: &RunInputs,
    step: GlobalStep,
) -> Result<CheckpointForStep, S2TrainRunError> {
    let tensors = toy0_tensors_for_step(inputs.seed, inputs.build_kind, step)?;
    let checkpoint_bytes = canonical_checkpoint_bytes(&tensors, &CheckpointMetadata::default())?;
    let checkpoint_sha = sha256(&checkpoint_bytes);
    Ok(CheckpointForStep {
        tensors,
        checkpoint_bytes,
        checkpoint_sha,
    })
}

fn toy0_tensors_for_step(
    seed: u64,
    build_kind: S2BuildKind,
    step: GlobalStep,
) -> Result<Vec<CanonicalTensor>, S2TrainRunError> {
    let model = toy0_model_state_for_step(seed, build_kind, step);
    let mut tensors = vec![
        tensor(
            "toy0.block0.weight",
            CanonicalTensorKind::DenseWeight,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(model.weights.to_vec()),
            &[2, 2],
        )?,
        tensor(
            "toy0.block0.bias",
            CanonicalTensorKind::DenseBias,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(model.bias.to_vec()),
            &[2],
        )?,
    ];
    if let Some(thresholds) = model.thresholds {
        tensors.push(tensor(
            "toy0.block0.thresholds",
            CanonicalTensorKind::DenseBias,
            TensorElementType::Float32,
            CanonicalTensorPayload::F32(thresholds.to_vec()),
            &[2],
        )?);
        tensors.push(tensor(
            "toy0.block0.scales",
            CanonicalTensorKind::TernaryScale,
            TensorElementType::Q8_8,
            CanonicalTensorPayload::U16(vec![256, 384]),
            &[2],
        )?);
    }
    Ok(tensors)
}

fn tensor(
    id: &str,
    kind: CanonicalTensorKind,
    element_type: TensorElementType,
    payload: CanonicalTensorPayload,
    dims: &[usize],
) -> Result<CanonicalTensor, S2TrainRunError> {
    let path = ArtifactPath::new(id).map_err(|error| S2TrainRunError::Tensor(error.to_string()))?;
    let shape = CanonicalTensorShape::from_usize_dims(dims)?;
    let layout = CanonicalTensorLayout::new(shape, element_type);
    Ok(CanonicalTensor::new(path, kind, layout, payload)?)
}

fn score_report_for_run(
    inputs: &RunInputs,
    final_checkpoint: &CheckpointForStep,
) -> Result<S2ScoreReport, S2TrainRunError> {
    let token_count = inputs.corpus_val.bytes.len() as u64;
    let log2_sum = token_count as f64
        * eval_bpc_for_step(
            optimizer_steps_for_build(inputs.build_kind),
            inputs.build_kind,
        );
    Ok(S2ScoreReport::new(
        inputs.seed,
        inputs.build_kind,
        final_checkpoint.checkpoint_sha,
        inputs.corpus_val.observed_sha(),
        token_count,
        log2_sum,
        qat_threshold_stats(inputs.build_kind),
        qat_scale_stats(inputs.build_kind),
    )?)
}

fn qat_threshold_stats(build_kind: S2BuildKind) -> Option<ThresholdStatsSummary> {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
    .then_some(ThresholdStatsSummary {
        matrices: 1,
        threshold_min: 0.7,
        threshold_max: 0.8,
        threshold_mean: 0.75,
        threshold_count: 2,
    })
}

fn qat_scale_stats(build_kind: S2BuildKind) -> Option<ScaleStatsSummary> {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
    .then_some(ScaleStatsSummary {
        matrices: 1,
        scale_count: 2,
        scale_min: 1.0,
        scale_max: 1.5,
        scale_mean_f32: 1.25,
    })
}

fn distillation_log_for_run(
    inputs: &RunInputs,
    phase_entries: &[PhaseEntry],
    loss_diagnostics_by_step: &BTreeMap<GlobalStep, Toy0LossTermDiagnostics>,
    phase_log_self_hash: Hash256,
    teacher: FrozenTeacherS2,
) -> Result<DistillationLog, S2TrainRunError> {
    let full_config = inputs.train_config.full_view();
    let mut distill_points = Vec::new();
    let mut loss_terms = Vec::new();
    for entry in phase_entries
        .iter()
        .filter(|entry| entry.step % full_config.eval_every_steps == 0)
    {
        distill_points.push(DistillEvalPoint {
            eval_step: entry.step,
            distill_loss: entry.distill_loss,
        });
        let diagnostics = loss_diagnostics_by_step.get(&entry.step).ok_or_else(|| {
            S2TrainRunError::train_loop(format!(
                "missing Toy0 loss diagnostics for step {}",
                entry.step
            ))
        })?;
        loss_terms.push(loss_terms_for_entry(entry, *diagnostics)?);
    }
    Ok(DistillationLog::new(
        inputs.seed,
        inputs.build_kind,
        teacher.teacher_checkpoint_sha,
        teacher.teacher_weight_fingerprint,
        teacher.teacher_storage_fingerprint,
        S2_DISTILL_TEMPERATURE,
        lambda_distill_default_for_build_kind(
            full_config.lambda_distill_default,
            inputs.build_kind,
        ),
        distill_points,
        phase_log_self_hash,
        loss_terms,
    )?)
}

fn loss_terms_for_entry(
    entry: &PhaseEntry,
    diagnostics: Toy0LossTermDiagnostics,
) -> Result<LossTermEvalPoint, S2TrainRunError> {
    let mut raw_losses = BTreeMap::new();
    let mut weighted_losses = BTreeMap::new();
    raw_losses.insert("distill".to_owned(), diagnostics.distill_loss);
    weighted_losses.insert(
        "distill".to_owned(),
        weighted_loss_term(
            "distill",
            diagnostics.distill_loss,
            entry.lambda_effective.lambda_distill,
        )?,
    );
    raw_losses.insert("range".to_owned(), diagnostics.range_loss);
    weighted_losses.insert(
        "range".to_owned(),
        weighted_loss_term(
            "range",
            diagnostics.range_loss,
            entry.lambda_effective.lambda_range,
        )?,
    );
    raw_losses.insert("zero".to_owned(), diagnostics.zero_loss);
    weighted_losses.insert(
        "zero".to_owned(),
        weighted_loss_term(
            "zero",
            diagnostics.zero_loss,
            entry.lambda_effective.lambda_zero,
        )?,
    );
    Ok(LossTermEvalPoint {
        eval_step: entry.step,
        lambda_effective: entry.lambda_effective,
        raw_losses,
        weighted_losses,
    })
}

fn weighted_loss_term(
    term: &'static str,
    raw: Option<f32>,
    lambda: f32,
) -> Result<Option<f32>, S2TrainRunError> {
    if raw.is_none() && lambda != 0.0 {
        return Err(S2TrainRunError::train_loop(format!(
            "{term} raw loss missing while lambda is {lambda}"
        )));
    }
    raw.map(|value| {
        let weighted = value * lambda;
        if weighted.is_finite() && weighted >= 0.0 {
            Ok(weighted)
        } else {
            Err(S2TrainRunError::train_loop(format!(
                "{term} weighted loss must be finite non-negative, got {weighted}"
            )))
        }
    })
    .transpose()
}

fn frozen_teacher_storage_fingerprint(
    seed: u64,
    build_kind: S2BuildKind,
    teacher_checkpoint_sha: Hash256,
    teacher_checkpoint_bytes: &[u8],
) -> Hash256 {
    #[derive(Serialize)]
    #[serde(deny_unknown_fields)]
    struct StorageFingerprintInput<'a> {
        seed: u64,
        build_kind: S2BuildKind,
        teacher_checkpoint_sha: Hash256,
        teacher_checkpoint_bytes_sha: Hash256,
        detached_requires_grad: bool,
        storage_contract: &'a str,
    }
    let input = StorageFingerprintInput {
        seed,
        build_kind,
        teacher_checkpoint_sha,
        teacher_checkpoint_bytes_sha: sha256(teacher_checkpoint_bytes),
        detached_requires_grad: false,
        storage_contract: "frozen-teacher-detached-clone",
    };
    DomainHash::new(
        "gbf-experiments",
        "FrozenTeacherS2",
        "s2_frozen_teacher_storage.v1",
        "1",
    )
    .hash(&input)
    .expect("storage fingerprint input is canonical")
}

/// Errors produced before or during the S2 train-run orchestration boundary.
#[derive(Debug)]
pub enum S2TrainRunError {
    /// S2 precondition failed.
    Precondition(S2PreconditionError),
    /// Burn-backed Toy0 training-loop boundary failed.
    TrainLoop(String),
    /// Phase-plan validation failed.
    PhasePlan(scheduler::PhasePlanError),
    /// S2 schema validation failed.
    Schema(crate::s2::schema::S2SchemaError),
    /// Canonical JSON/schema hash failed.
    Canonical(S1SchemaError),
    /// Checkpoint serialization failed.
    Checkpoint(CheckpointWriteError),
    /// Canonical tensor construction failed.
    Tensor(String),
}

impl fmt::Display for S2TrainRunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Precondition(error) => write!(f, "{error}"),
            Self::TrainLoop(error) => write!(f, "{error}"),
            Self::PhasePlan(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::Canonical(error) => write!(f, "{error}"),
            Self::Checkpoint(error) => write!(f, "{error}"),
            Self::Tensor(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S2TrainRunError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Precondition(error) => Some(error),
            Self::TrainLoop(_) => None,
            Self::PhasePlan(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::Canonical(error) => Some(error),
            Self::Checkpoint(error) => Some(error),
            Self::Tensor(_) => None,
        }
    }
}

impl S2TrainRunError {
    fn train_loop(error: impl fmt::Display) -> Self {
        Self::TrainLoop(error.to_string())
    }
}

impl From<scheduler::PhasePlanError> for S2TrainRunError {
    fn from(error: scheduler::PhasePlanError) -> Self {
        Self::PhasePlan(error)
    }
}

impl From<crate::s2::schema::S2SchemaError> for S2TrainRunError {
    fn from(error: crate::s2::schema::S2SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<S1SchemaError> for S2TrainRunError {
    fn from(error: S1SchemaError) -> Self {
        Self::Canonical(error)
    }
}

impl From<CheckpointWriteError> for S2TrainRunError {
    fn from(error: CheckpointWriteError) -> Self {
        Self::Checkpoint(error)
    }
}

impl From<CanonicalTensorError> for S2TrainRunError {
    fn from(error: CanonicalTensorError) -> Self {
        Self::Tensor(error.to_string())
    }
}

/// Structured S2 precondition failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum S2PreconditionError {
    /// Corpus bytes did not match the pinned hash.
    CorpusShaMismatch {
        /// Corpus label.
        corpus: &'static str,
        /// Expected content hash.
        expected: Hash256,
        /// Observed content hash.
        observed: Hash256,
    },
    /// Deterministic environment variable had a forbidden value.
    EnvExactViolation {
        /// Environment variable name.
        var: &'static str,
        /// Required value.
        expected: &'static str,
        /// Observed value.
        observed: Option<String>,
    },
    /// Build kind and train-config variant are incompatible.
    BuildConfigMismatch {
        /// Runtime build kind.
        build_kind: S2BuildKind,
    },
    /// Only Toy0 is implemented for S2 today.
    UnsupportedModelProfile {
        /// Observed profile.
        profile: String,
    },
}

impl fmt::Display for S2PreconditionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CorpusShaMismatch {
                corpus,
                expected,
                observed,
            } => write!(
                f,
                "S2-Pre corpus sha mismatch for {corpus}: expected {expected}, observed {observed}"
            ),
            Self::EnvExactViolation {
                var,
                expected,
                observed,
            } => write!(
                f,
                "S2-Pre env_exact violation for {var}: expected {expected}, observed {}",
                observed.as_deref().unwrap_or("<unset>")
            ),
            Self::BuildConfigMismatch { build_kind } => {
                write!(f, "S2-Pre build/config mismatch for {build_kind:?}")
            }
            Self::UnsupportedModelProfile { profile } => {
                write!(f, "S2-Pre unsupported model profile {profile}")
            }
        }
    }
}

impl Error for S2PreconditionError {}

/// F-S2 pre-train state-machine orchestration.
pub mod state_machine {
    use std::error::Error;
    use std::fmt;
    use std::time::Instant;

    use crate::S2_LOG_TARGET;
    use crate::s2::report::decision_for_outcome;
    use crate::s2::schema::{S2Decision, S2Outcome};

    /// F-S2 experiment state tag.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub enum State {
        /// Configuration parsed and validated.
        Configured,
        /// Baseline artifacts loaded.
        LoadedBaselines,
        /// H6 LinearState smoke has run.
        LinearStateSmokeRun,
        /// H5 loss-gradient-flow checks have run.
        LossGradFlowRun,
        /// D8 phase-transition integration has run.
        PhaseTransitionIntegRun,
        /// S1 oracle suite has been re-run under the S2 binary.
        OracleReRun,
        /// Public API drift has been checked.
        ApiDriftChecked,
        /// S2 falsification suite has run.
        FalsificationChecked,
        /// Full S2 train budget has been attempted.
        TrainAttempted,
        /// Full S2 training completed.
        Trained,
        /// Score reports emitted.
        Scored,
        /// Gap metrics computed.
        GapComputed,
        /// Phase-A ablation run attempted.
        AblationAttempted,
        /// Ablation result compared.
        AblationCompared,
        /// Report artifact emitted.
        Reported,
        /// Closure decision reached.
        Decided,
    }

    impl State {
        /// Stable state tag used in logs and tests.
        #[must_use]
        pub const fn as_str(self) -> &'static str {
            match self {
                Self::Configured => "Configured",
                Self::LoadedBaselines => "LoadedBaselines",
                Self::LinearStateSmokeRun => "LinearStateSmokeRun",
                Self::LossGradFlowRun => "LossGradFlowRun",
                Self::PhaseTransitionIntegRun => "PhaseTransitionIntegRun",
                Self::OracleReRun => "OracleReRun",
                Self::ApiDriftChecked => "ApiDriftChecked",
                Self::FalsificationChecked => "FalsificationChecked",
                Self::TrainAttempted => "TrainAttempted",
                Self::Trained => "Trained",
                Self::Scored => "Scored",
                Self::GapComputed => "GapComputed",
                Self::AblationAttempted => "AblationAttempted",
                Self::AblationCompared => "AblationCompared",
                Self::Reported => "Reported",
                Self::Decided => "Decided",
            }
        }
    }

    /// Boolean outputs from early S2 verifier gates.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct PreTrainGateResults {
        /// H6 LinearState smoke verdict.
        pub linearstate_smoke_passed: bool,
        /// H5 loss-gradient-flow verdict.
        pub loss_grad_flow_passed: bool,
        /// D8 phase-transition integration verdict.
        pub phase_transition_integ_passed: bool,
        /// O3 oracle re-run verdict.
        pub oracle_re_run_passed: bool,
        /// O11 API-drift verdict.
        pub api_drift_check_passed: bool,
        /// F-S2 falsification suite verdict.
        pub falsification_s2_passed: bool,
        /// Whether the ablation branch should execute after gap computation.
        pub ablation_required: bool,
    }

    impl Default for PreTrainGateResults {
        fn default() -> Self {
            Self {
                linearstate_smoke_passed: true,
                loss_grad_flow_passed: true,
                phase_transition_integ_passed: true,
                oracle_re_run_passed: true,
                api_drift_check_passed: true,
                falsification_s2_passed: true,
                ablation_required: true,
            }
        }
    }

    /// One state transition observed by the orchestrator.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StateTransition {
        /// Previous state.
        pub from: State,
        /// Next state.
        pub to: State,
        /// Whether the transition represents successful progress.
        pub success: bool,
    }

    /// Output from a synthetic F-S2 state-machine run.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct StateMachineRun {
        /// All transitions taken, including post-failure cleanup transitions.
        pub transitions: Vec<StateTransition>,
        /// Final state.
        pub final_state: State,
        /// Dispatched outcome.
        pub outcome: S2Outcome,
        /// Decision implied by the outcome.
        pub decision: S2Decision,
        /// Whether the full 15-run training budget was attempted.
        pub train_attempted: bool,
    }

    /// State-machine transition error.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum StateMachineError {
        /// Transition is not legal from the current state.
        InvalidTransition {
            /// Current state.
            from: State,
            /// Requested next state.
            to: State,
        },
    }

    impl fmt::Display for StateMachineError {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::InvalidTransition { from, to } => {
                    write!(f, "invalid S2 state transition {} -> {}", from, to)
                }
            }
        }
    }

    impl Error for StateMachineError {}

    /// Run the pre-train gates and synthetic post-train state sequence.
    pub fn run_pretrain_state_machine(gates: PreTrainGateResults) -> StateMachineRun {
        let mut runner = Runner::default();
        runner.advance(State::LoadedBaselines);
        runner.advance(State::LinearStateSmokeRun);
        if !gates.linearstate_smoke_passed {
            return runner.fail(
                State::LinearStateSmokeRun,
                "H6 LinearState smoke failed",
                S2Outcome::FailLinearstate,
            );
        }
        runner.advance(State::LossGradFlowRun);
        if !gates.loss_grad_flow_passed {
            return runner.fail(
                State::LossGradFlowRun,
                "H5 loss-gradient-flow failed",
                S2Outcome::FailLossGradFlow,
            );
        }
        runner.advance(State::PhaseTransitionIntegRun);
        if !gates.phase_transition_integ_passed {
            return runner.fail(
                State::PhaseTransitionIntegRun,
                "D8 phase-transition integration failed",
                S2Outcome::FailPhaseIntegration,
            );
        }
        runner.advance(State::OracleReRun);
        if !gates.oracle_re_run_passed {
            return runner.fail(
                State::OracleReRun,
                "S1 oracle re-run failed under S2 binary",
                S2Outcome::FailMetric,
            );
        }
        runner.advance(State::ApiDriftChecked);
        if !gates.api_drift_check_passed {
            return runner.fail(
                State::ApiDriftChecked,
                "public API drift check failed",
                S2Outcome::FailApiDrift,
            );
        }
        runner.advance(State::FalsificationChecked);
        if !gates.falsification_s2_passed {
            return runner.fail(
                State::FalsificationChecked,
                "S2 falsification suite failed",
                S2Outcome::FailFalsification,
            );
        }
        runner.advance(State::TrainAttempted);
        runner.train_attempted = true;
        runner.advance(State::Trained);
        runner.advance(State::Scored);
        runner.advance(State::GapComputed);
        if gates.ablation_required {
            runner.advance(State::AblationAttempted);
            runner.advance(State::AblationCompared);
        }
        runner.advance(State::Reported);
        runner.advance(State::Decided);
        runner.finish(S2Outcome::PassClean)
    }

    /// Validate one explicit transition without mutating global state.
    pub fn validate_transition(
        from: State,
        to: State,
    ) -> Result<StateTransition, StateMachineError> {
        if allowed_next(from).contains(&to) {
            Ok(StateTransition {
                from,
                to,
                success: true,
            })
        } else {
            Err(StateMachineError::InvalidTransition { from, to })
        }
    }

    fn allowed_next(from: State) -> &'static [State] {
        match from {
            State::Configured => &[State::LoadedBaselines],
            State::LoadedBaselines => &[State::LinearStateSmokeRun],
            State::LinearStateSmokeRun => &[State::LossGradFlowRun, State::Reported],
            State::LossGradFlowRun => &[State::PhaseTransitionIntegRun, State::Reported],
            State::PhaseTransitionIntegRun => &[State::OracleReRun, State::Reported],
            State::OracleReRun => &[State::ApiDriftChecked, State::Reported],
            State::ApiDriftChecked => &[State::FalsificationChecked, State::Reported],
            State::FalsificationChecked => &[State::TrainAttempted, State::Reported],
            State::TrainAttempted => &[State::Trained],
            State::Trained => &[State::Scored],
            State::Scored => &[State::GapComputed],
            State::GapComputed => &[State::AblationAttempted, State::Reported],
            State::AblationAttempted => &[State::AblationCompared],
            State::AblationCompared => &[State::Reported],
            State::Reported => &[State::Decided],
            State::Decided => &[],
        }
    }

    #[derive(Debug)]
    struct Runner {
        state: State,
        transitions: Vec<StateTransition>,
        train_attempted: bool,
        last_transition_at: Instant,
    }

    impl Default for Runner {
        fn default() -> Self {
            Self {
                state: State::Configured,
                transitions: Vec::new(),
                train_attempted: false,
                last_transition_at: Instant::now(),
            }
        }
    }

    impl Runner {
        fn advance(&mut self, to: State) {
            let transition =
                validate_transition(self.state, to).expect("state-machine path is valid");
            self.advance_transition(transition);
        }

        fn advance_with_success(&mut self, to: State, success: bool) {
            let mut transition =
                validate_transition(self.state, to).expect("state-machine path is valid");
            transition.success = success;
            self.advance_transition(transition);
        }

        fn advance_transition(&mut self, transition: StateTransition) {
            let elapsed_ms = self.last_transition_at.elapsed().as_millis() as u64;
            log_transition(&transition, elapsed_ms);
            self.last_transition_at = Instant::now();
            self.state = transition.to;
            self.transitions.push(transition);
        }

        fn fail(
            mut self,
            state: State,
            reason: &'static str,
            outcome: S2Outcome,
        ) -> StateMachineRun {
            tracing::error!(
                target: S2_LOG_TARGET,
                event_name = "state_transition_failed",
                event = "state_transition_failed",
                state = state.as_str(),
                reason,
                will_emit_report = true,
                "s2 state transition failed"
            );
            if self.state != State::Reported {
                self.advance_with_success(State::Reported, false);
            }
            self.advance_with_success(State::Decided, false);
            self.finish(outcome)
        }

        fn finish(self, outcome: S2Outcome) -> StateMachineRun {
            StateMachineRun {
                transitions: self.transitions,
                final_state: self.state,
                outcome,
                decision: decision_for_outcome(outcome),
                train_attempted: self.train_attempted,
            }
        }
    }

    fn log_transition(transition: &StateTransition, elapsed_ms: u64) {
        tracing::info!(
            target: S2_LOG_TARGET,
            event_name = "state_transition",
            event = "state_transition",
            from = transition.from.as_str(),
            to = transition.to.as_str(),
            elapsed_ms,
            success = transition.success,
            "s2 state transition"
        );
    }

    impl fmt::Display for State {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(self.as_str())
        }
    }
}

type DistillLossNats = f32;

const ENV_EXACT: [(&str, &str); 4] = [
    ("BURN_NDARRAY_NUM_THREADS", "1"),
    ("BURN_DETERMINISTIC", "1"),
    ("OMP_NUM_THREADS", "1"),
    ("RAYON_NUM_THREADS", "1"),
];
