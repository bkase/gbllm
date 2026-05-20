//! S4 Gutenberg continuation run surface.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use gbf_foundation::Hash256;
use gbf_model::qat::RouterTrainMode;
use gbf_train::phase::{PhaseScheduleError, QuantHardness, TrainPhaseKind, TrainPhaseSpec};

use crate::S4_LOG_TARGET;
use crate::s4::schema::{
    S4_CANONICAL_SEEDS, S4_OPTIMIZER_STEPS_GUTENBERG, S4InitialWeightSource,
    S4OptimizerStateInitial, S4QatShadowWeightSource, S4SchemaError, S4SeedRunContract,
    S4TernaryProjectionInitial, S4TrainConfig, S4TrainPhase, train_config_hash,
    validate_s4_canonical_seed_list, validate_s4_seed,
};

/// Structured S4 step event name emitted by the run-loop evidence helper.
pub const S4_STEP_EVENT_NAME: &str = "s4_step";

/// Structured event emitted when D9 continuation initialization begins.
pub const S4_CONTINUATION_INIT_STARTED_EVENT_NAME: &str = "s4_continuation_init_started";

/// Structured event emitted when the D9 contract resumes directly into Phase D.
pub const S4_RESUMED_PHASE_D_EVENT_NAME: &str = "s4_resumed_phase_d";

/// Structured event emitted when D9 resets AdamW optimizer state.
pub const S4_OPTIMIZER_STATE_ZEROED_EVENT_NAME: &str = "s4_optimizer_state_zeroed";

/// Structured event emitted when the D9 BatchRng descriptor is instantiated.
pub const S4_BATCHRNG_INSTANTIATED_EVENT_NAME: &str = "s4_batchrng_instantiated";

const S4_ACTUAL_LOAD_SCOPE_LABEL: &str = "contract_pin_only";
const S4_INITIAL_WEIGHT_SOURCE_LABEL: &str = "c_TS_ref";
const S4_QAT_SHADOW_WEIGHT_SOURCE_LABEL: &str = "c_TS_ref";
const S4_OPTIMIZER_STATE_INITIAL_LABEL: &str = "zero_init_adamw";
const S4_PHASE_STATE_INITIAL_LABEL: &str = "phase_d";
const S4_TERNARY_PROJECTION_INITIAL_LABEL: &str = "phase_d_hard_ternary_projection";
const S4_INHERITED_ADAMW_MOMENT_LABEL_NONE: &str = "none";
const S4_ADAMW_MOMENT_INHERITANCE_PROOF_LABEL: &str = "impossible_by_input_contract";

/// Inputs needed to initialize one D9 Gutenberg continuation run.
///
/// This input type intentionally has no inherited AdamW moment fields. D9 is a
/// warm-weight, cold-optimizer corpus boundary: the promoted c_TS payloads are
/// hash-pinned here, while optimizer moments from TinyStories cannot be
/// supplied to this constructor at all.
#[derive(Debug, Clone, PartialEq)]
pub struct S4ContinuationInitInputs {
    /// Gutenberg continuation seed from D11.
    pub seed: u64,
    /// Pinned D9/D10 train configuration.
    pub train_config: S4TrainConfig,
    /// Promoted S3 ternary checkpoint self-hash.
    pub c_ts_checkpoint_self_hash: Hash256,
    /// Canonical tensor payload SHA for deployed ternary weights loaded from c_TS.
    pub deployed_tensor_payload_sha: Hash256,
    /// Canonical tensor payload SHA for FP/QAT shadow weights loaded from c_TS.
    pub fp_shadow_tensor_payload_sha: Hash256,
    /// Self-hash of the promotion gate that accepted c_TS.
    pub promotion_gate_self_hash: Hash256,
}

/// D9 initialization evidence for one seed before optimizer step 1.
///
/// This is contract-pin evidence for the future runner. It records the promoted
/// checkpoint and tensor payload hashes that the runner must load; it does not
/// perform the actual safetensor read in this helper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4ContinuationInitialization {
    /// Gutenberg continuation seed from D11.
    pub seed: u64,
    /// D10 train-config hash shared by all canonical seeds.
    pub train_config_hash: Hash256,
    /// Promoted S3 ternary checkpoint self-hash.
    pub c_ts_checkpoint_self_hash: Hash256,
    /// Promotion-gate self-hash that authorized the c_TS lineage.
    pub promotion_gate_self_hash: Hash256,
    /// Run-log lineage field for deployed weights loaded as `model_weights_initial`.
    pub initial_checkpoint_payload_sha: Hash256,
    /// Run-log lineage field for QAT FP shadow weights loaded for Phase-D resume.
    pub initial_fp_shadow_payload_sha: Hash256,
    /// D9 deployed-weight source; must remain `c_TS_ref`.
    pub initial_weight_source: S4InitialWeightSource,
    /// D9 QAT shadow-weight source; absence is a promotion-gate failure.
    pub qat_shadow_weights_initial: S4QatShadowWeightSource,
    /// D9 cold optimizer-state initialization.
    pub optimizer_state_initial: S4OptimizerStateInitial,
    /// Optimizer step before the first Gutenberg update.
    pub optimizer_step_initial: u64,
    /// Always `None`; inherited AdamW first moments cannot be supplied through
    /// [`S4ContinuationInitInputs`].
    pub inherited_adamw_first_moment_payload_sha: Option<Hash256>,
    /// Always `None`; inherited AdamW second moments cannot be supplied through
    /// [`S4ContinuationInitInputs`].
    pub inherited_adamw_second_moment_payload_sha: Option<Hash256>,
    /// D9 continuation phase state.
    pub phase_state_initial: S4TrainPhase,
    /// D9 Phase-D hard ternary projection at continuation entry.
    pub ternary_projection_initial: S4TernaryProjectionInitial,
    /// Per-seed RNG stream descriptors initialized for S4.
    pub rng_streams: crate::s4::rng::S4RngStreams,
    /// D9 requires InitRng to consume zero draws before optimizer step 1.
    pub init_rng_draw_count_before_first_step: u64,
    /// BatchRng is initialized but not consumed during initialization.
    pub batch_rng_draw_count_before_first_step: u64,
    /// S4 v1 reserves ShuffleRng and consumes zero draws total.
    pub shuffle_rng_draw_count_total: u64,
}

/// Initialize D9 warm-weight/cold-optimizer Gutenberg continuation evidence.
///
/// The helper pins lineage and run-start contracts only. Actual tensor loading
/// from the referenced c_TS artifacts is owned by the later runner/artifact
/// path, so emitted events name the scope as contract-pin-only.
pub fn initialize_gutenberg_continuation(
    inputs: &S4ContinuationInitInputs,
) -> Result<S4ContinuationInitialization, S4RunScheduleError> {
    validate_s4_seed(inputs.seed)?;
    validate_nonzero_initial_hash(
        "c_ts_checkpoint_self_hash",
        inputs.c_ts_checkpoint_self_hash,
    )?;
    validate_nonzero_initial_hash(
        "deployed_tensor_payload_sha",
        inputs.deployed_tensor_payload_sha,
    )?;
    validate_nonzero_initial_hash(
        "fp_shadow_tensor_payload_sha",
        inputs.fp_shadow_tensor_payload_sha,
    )?;
    validate_nonzero_initial_hash("promotion_gate_self_hash", inputs.promotion_gate_self_hash)?;

    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = S4_CONTINUATION_INIT_STARTED_EVENT_NAME,
        seed = inputs.seed,
        c_ts_checkpoint_self_hash = %inputs.c_ts_checkpoint_self_hash,
        deployed_tensor_payload_sha = %inputs.deployed_tensor_payload_sha,
        fp_shadow_tensor_payload_sha = %inputs.fp_shadow_tensor_payload_sha,
        promotion_gate_self_hash = %inputs.promotion_gate_self_hash,
        actual_load_scope = S4_ACTUAL_LOAD_SCOPE_LABEL,
        actual_load_performed = false,
        "s4 continuation initialization started"
    );

    let train_config_hash = train_config_hash(&inputs.train_config)?;
    let contract = S4SeedRunContract::new(inputs.seed, train_config_hash)?;

    let init = S4ContinuationInitialization {
        seed: inputs.seed,
        train_config_hash,
        c_ts_checkpoint_self_hash: inputs.c_ts_checkpoint_self_hash,
        promotion_gate_self_hash: inputs.promotion_gate_self_hash,
        initial_checkpoint_payload_sha: inputs.deployed_tensor_payload_sha,
        initial_fp_shadow_payload_sha: inputs.fp_shadow_tensor_payload_sha,
        initial_weight_source: contract.initial_weight_source,
        qat_shadow_weights_initial: contract.qat_shadow_weights_initial,
        optimizer_state_initial: contract.optimizer_state_initial,
        optimizer_step_initial: 0,
        inherited_adamw_first_moment_payload_sha: None,
        inherited_adamw_second_moment_payload_sha: None,
        phase_state_initial: contract.phase_state_initial,
        ternary_projection_initial: contract.ternary_projection_initial,
        rng_streams: contract.rng_streams,
        init_rng_draw_count_before_first_step: contract.init_rng_draw_count_before_first_step,
        batch_rng_draw_count_before_first_step: 0,
        shuffle_rng_draw_count_total: contract.shuffle_rng_draw_count_total,
    };

    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = S4_RESUMED_PHASE_D_EVENT_NAME,
        seed = init.seed,
        train_config_hash = %init.train_config_hash,
        c_ts_checkpoint_self_hash = %init.c_ts_checkpoint_self_hash,
        initial_checkpoint_payload_sha = %init.initial_checkpoint_payload_sha,
        initial_fp_shadow_payload_sha = %init.initial_fp_shadow_payload_sha,
        promotion_gate_self_hash = %init.promotion_gate_self_hash,
        initial_weight_source = S4_INITIAL_WEIGHT_SOURCE_LABEL,
        qat_shadow_weights_initial = S4_QAT_SHADOW_WEIGHT_SOURCE_LABEL,
        phase_state_initial = S4_PHASE_STATE_INITIAL_LABEL,
        ternary_projection_initial = S4_TERNARY_PROJECTION_INITIAL_LABEL,
        actual_load_scope = S4_ACTUAL_LOAD_SCOPE_LABEL,
        actual_load_performed = false,
        "s4 resumed phase d"
    );
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = S4_OPTIMIZER_STATE_ZEROED_EVENT_NAME,
        seed = init.seed,
        optimizer_state_initial = S4_OPTIMIZER_STATE_INITIAL_LABEL,
        optimizer_step_initial = init.optimizer_step_initial,
        inherited_adamw_first_moment_payload_sha = S4_INHERITED_ADAMW_MOMENT_LABEL_NONE,
        inherited_adamw_second_moment_payload_sha = S4_INHERITED_ADAMW_MOMENT_LABEL_NONE,
        adamw_moment_inheritance_proof = S4_ADAMW_MOMENT_INHERITANCE_PROOF_LABEL,
        "s4 optimizer state zeroed"
    );
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = S4_BATCHRNG_INSTANTIATED_EVENT_NAME,
        seed = init.seed,
        domain = init.rng_streams.batch.domain.as_str(),
        seed128_hex = init.rng_streams.batch.seed128_hex.as_str(),
        initial_state_hex = init.rng_streams.batch.initial_state_hex.as_str(),
        draw_count = init.rng_streams.batch.draw_count,
        batch_rng_draw_count_before_first_step = init.batch_rng_draw_count_before_first_step,
        "s4 batch rng instantiated"
    );

    Ok(init)
}

/// Reject missing D9 lineage hashes before emitting initialization evidence.
///
/// `Hash256::ZERO` is reserved as the local missing-value sentinel for
/// unbound artifact and payload hashes; it is not accepted as a real
/// c_TS/promotion-gate lineage hash.
fn validate_nonzero_initial_hash(
    field: &'static str,
    hash: Hash256,
) -> Result<(), S4RunScheduleError> {
    if hash == Hash256::ZERO {
        Err(S4RunScheduleError::MissingInitialHash { field })
    } else {
        Ok(())
    }
}

/// Validated one-phase descriptor for D9 Phase-D continuation.
///
/// The production runner that consumes this descriptor is owned by a later S4
/// bead. This type pins the contract that every Gutenberg optimizer step uses
/// fully hardened `gbf-train` Phase-D controls without rerunning A/B/C warmup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4PhaseDContinuationSchedule {
    phase: TrainPhaseSpec,
}

impl S4PhaseDContinuationSchedule {
    /// Construct the Phase-D continuation descriptor from a validated config.
    pub fn new(config: &S4TrainConfig) -> Result<Self, S4RunScheduleError> {
        config.validate()?;
        let phase = TrainPhaseSpec::new(
            TrainPhaseKind::FullNumericQat,
            0,
            config.optimizer_steps,
            QuantHardness::Hard,
            QuantHardness::Hard,
            QuantHardness::Hard,
            RouterTrainMode::HardTop1,
        )?;
        Ok(Self { phase })
    }

    /// Construct the pinned D9/D10 Phase-D continuation descriptor.
    pub fn pinned() -> Result<Self, S4RunScheduleError> {
        Self::new(&S4TrainConfig::pinned())
    }

    /// Return the underlying `gbf-train` Phase-D specification.
    #[must_use]
    pub const fn phase(&self) -> TrainPhaseSpec {
        self.phase
    }

    /// Map a 1-indexed Gutenberg optimizer step into the Phase-D descriptor.
    pub fn phase_for_optimizer_step(
        &self,
        optimizer_step: u64,
    ) -> Result<TrainPhaseSpec, S4RunScheduleError> {
        self.scheduler_step_for_optimizer_step(optimizer_step)?;
        Ok(self.phase)
    }

    /// Convert a 1-indexed optimizer step into the 0-indexed scheduler step.
    pub fn scheduler_step_for_optimizer_step(
        &self,
        optimizer_step: u64,
    ) -> Result<u64, S4RunScheduleError> {
        let max_step = self.phase.len_steps();
        if optimizer_step == 0 || optimizer_step > max_step {
            return Err(S4RunScheduleError::StepOutOfRange {
                step: optimizer_step,
                max_step,
            });
        }
        Ok(optimizer_step - 1)
    }
}

/// Return the progress-evaluation steps required by S4-Run-Ok-2.
pub fn progress_eval_steps(config: &S4TrainConfig) -> Result<Vec<u64>, S4RunScheduleError> {
    config.validate()?;
    let mut steps = Vec::new();
    let mut step = 0_u64;
    while step <= config.optimizer_steps {
        steps.push(step);
        step = step
            .checked_add(config.eval_every_steps)
            .ok_or(S4RunScheduleError::StepOverflow)?;
    }
    if *steps.last().expect("step 0 is always pushed") != config.optimizer_steps {
        return Err(S4RunScheduleError::EvalCadenceDoesNotLandOnFinalStep {
            optimizer_steps: config.optimizer_steps,
            eval_every_steps: config.eval_every_steps,
        });
    }
    Ok(steps)
}

/// Build order-preserving per-seed contracts without sharing mutable state.
///
/// This helper accepts any non-empty duplicate-free subset of the D11 seeds so
/// smoke determinism tests can compare `[0, 1]` and `[1, 0]`. Closure-candidate
/// runs must use [`canonical_seed_run_contracts`].
pub fn seed_run_contracts_for_ordered_seeds(
    seeds: &[u64],
    config: &S4TrainConfig,
) -> Result<Vec<S4SeedRunContract>, S4RunScheduleError> {
    if seeds.is_empty() {
        return Err(S4RunScheduleError::EmptySeedList);
    }

    let mut observed = BTreeSet::new();
    for &seed in seeds {
        if !observed.insert(seed) {
            return Err(S4RunScheduleError::DuplicateSeed { seed });
        }
    }

    let hash = train_config_hash(config)?;
    seeds
        .iter()
        .copied()
        .map(|seed| S4SeedRunContract::new(seed, hash).map_err(S4RunScheduleError::from))
        .collect()
}

/// Build the exact D11 five-seed closure-candidate contracts.
pub fn canonical_seed_run_contracts(
    config: &S4TrainConfig,
) -> Result<Vec<S4SeedRunContract>, S4RunScheduleError> {
    validate_s4_canonical_seed_list(&S4_CANONICAL_SEEDS)?;
    seed_run_contracts_for_ordered_seeds(&S4_CANONICAL_SEEDS, config)
}

/// One dynamic S4 optimizer-step evidence record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4StepEvent {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// 1-indexed optimizer step emitted by the S4 run loop.
    pub step: u64,
    /// 0-indexed scheduler step consumed by the Phase-D descriptor.
    pub scheduler_step: u64,
    /// S4 phase state for this step.
    pub phase: S4TrainPhase,
}

impl S4StepEvent {
    /// Compact phase label required by `s4_step` structured events.
    #[must_use]
    pub const fn phase_label(&self) -> &'static str {
        match self.phase {
            S4TrainPhase::PhaseD => "D",
        }
    }
}

/// Dynamic S4 run-loop evidence for one seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct S4RunLoopEvidence {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Number of optimizer steps completed by the helper.
    pub completed_optimizer_steps: u64,
    /// 1-indexed final optimizer step reached by the helper, if any.
    pub final_optimizer_step: Option<u64>,
    /// Ordered per-step Phase-D evidence emitted by the helper.
    pub event_history: Vec<S4StepEvent>,
}

/// Stateful S4 run-loop helper that keeps per-seed optimizer-step progress.
#[derive(Debug, Clone)]
pub struct S4SeedRunLoop {
    seed: u64,
    optimizer_step_budget: u64,
    completed_optimizer_steps: u64,
    schedule: S4PhaseDContinuationSchedule,
    event_history: Vec<S4StepEvent>,
}

impl S4SeedRunLoop {
    /// Construct an independent run-loop state machine for one S4 seed.
    pub fn new(seed: u64, config: &S4TrainConfig) -> Result<Self, S4RunScheduleError> {
        validate_s4_seed(seed)?;
        let schedule = S4PhaseDContinuationSchedule::new(config)?;
        Ok(Self {
            seed,
            optimizer_step_budget: config.optimizer_steps,
            completed_optimizer_steps: 0,
            schedule,
            event_history: Vec::new(),
        })
    }

    /// Gutenberg continuation seed owned by this state machine.
    #[must_use]
    pub const fn seed(&self) -> u64 {
        self.seed
    }

    /// Number of optimizer steps already completed for this seed.
    #[must_use]
    pub const fn completed_optimizer_steps(&self) -> u64 {
        self.completed_optimizer_steps
    }

    /// Ordered Phase-D event history accumulated so far.
    #[must_use]
    pub fn event_history(&self) -> &[S4StepEvent] {
        &self.event_history
    }

    /// Advance exactly one optimizer step and emit an `s4_step` trace event.
    pub fn advance_one_step(&mut self) -> Result<S4StepEvent, S4RunScheduleError> {
        let requested_step = self
            .completed_optimizer_steps
            .checked_add(1)
            .ok_or(S4RunScheduleError::StepOverflow)?;
        if requested_step > self.optimizer_step_budget {
            let error = S4RunScheduleError::OptimizerStepBudgetExceeded {
                seed: self.seed,
                requested_step,
                max_step: self.optimizer_step_budget,
            };
            tracing::warn!(
                target: S4_LOG_TARGET,
                event_name = "s4_step_refused",
                error_name = error.name(),
                seed = self.seed,
                requested_step,
                max_step = self.optimizer_step_budget,
                "s4 optimizer step refused"
            );
            return Err(error);
        }

        let scheduler_step = self
            .schedule
            .scheduler_step_for_optimizer_step(requested_step)?;
        let event = S4StepEvent {
            seed: self.seed,
            step: requested_step,
            scheduler_step,
            phase: S4TrainPhase::PhaseD,
        };
        tracing::debug!(
            target: S4_LOG_TARGET,
            event_name = S4_STEP_EVENT_NAME,
            seed = event.seed,
            step = event.step,
            phase = event.phase_label(),
            scheduler_step = event.scheduler_step,
            "s4 optimizer step"
        );
        self.completed_optimizer_steps = requested_step;
        self.event_history.push(event.clone());
        Ok(event)
    }

    /// Advance until the exact D10 optimizer-step budget has been reached.
    pub fn run_to_budget(&mut self) -> Result<S4RunLoopEvidence, S4RunScheduleError> {
        while self.completed_optimizer_steps < self.optimizer_step_budget {
            self.advance_one_step()?;
        }
        Ok(self.evidence())
    }

    /// Snapshot current dynamic run-loop evidence.
    #[must_use]
    pub fn evidence(&self) -> S4RunLoopEvidence {
        S4RunLoopEvidence {
            seed: self.seed,
            completed_optimizer_steps: self.completed_optimizer_steps,
            final_optimizer_step: self.event_history.last().map(|event| event.step),
            event_history: self.event_history.clone(),
        }
    }
}

/// Number of train losses required by S4-RL-Length.
#[must_use]
pub const fn required_train_loss_count() -> u64 {
    S4_OPTIMIZER_STEPS_GUTENBERG
}

/// Errors raised by S4 run-schedule contract helpers.
#[derive(Debug, PartialEq)]
pub enum S4RunScheduleError {
    /// S4 config validation failed.
    Config(S4SchemaError),
    /// `gbf-train` phase spec construction failed.
    PhaseSchedule(PhaseScheduleError),
    /// The requested optimizer step is outside the D10 budget.
    StepOutOfRange {
        /// 1-indexed optimizer step.
        step: u64,
        /// Largest valid 1-indexed optimizer step.
        max_step: u64,
    },
    /// Progress-eval cadence did not land on the final optimizer step.
    EvalCadenceDoesNotLandOnFinalStep {
        /// Configured optimizer steps.
        optimizer_steps: u64,
        /// Configured eval cadence.
        eval_every_steps: u64,
    },
    /// Step arithmetic overflowed.
    StepOverflow,
    /// Seed-order helpers require at least one seed.
    EmptySeedList,
    /// Seed-order helpers reject duplicate seeds to keep contracts unambiguous.
    DuplicateSeed {
        /// Duplicated seed.
        seed: u64,
    },
    /// A per-seed run-loop tried to advance beyond the D10 optimizer budget.
    OptimizerStepBudgetExceeded {
        /// Gutenberg continuation seed.
        seed: u64,
        /// 1-indexed optimizer step that was refused.
        requested_step: u64,
        /// Largest valid 1-indexed optimizer step.
        max_step: u64,
    },
    /// D9 initialization requires explicit non-zero artifact/payload hashes.
    MissingInitialHash {
        /// Missing or zero hash field.
        field: &'static str,
    },
}

impl S4RunScheduleError {
    /// Stable structured error name for S4 run-schedule failures.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Config(_) => "S4ConfigError",
            Self::PhaseSchedule(_) => "S4PhaseScheduleError",
            Self::StepOutOfRange { .. } => "S4StepOutOfRange",
            Self::EvalCadenceDoesNotLandOnFinalStep { .. } => "S4EvalCadenceDoesNotLandOnFinalStep",
            Self::StepOverflow => "S4StepOverflow",
            Self::EmptySeedList => "S4EmptySeedList",
            Self::DuplicateSeed { .. } => "S4DuplicateSeed",
            Self::OptimizerStepBudgetExceeded { .. } => "S4OptimizerStepBudgetExceeded",
            Self::MissingInitialHash { .. } => "S4MissingInitialHash",
        }
    }
}

impl fmt::Display for S4RunScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(error) => write!(f, "{error}"),
            Self::PhaseSchedule(error) => write!(f, "{error}"),
            Self::StepOutOfRange { step, max_step } => {
                write!(f, "S4 optimizer step {step} is outside 1..={max_step}")
            }
            Self::EvalCadenceDoesNotLandOnFinalStep {
                optimizer_steps,
                eval_every_steps,
            } => write!(
                f,
                "S4 eval cadence {eval_every_steps} does not land on final step {optimizer_steps}"
            ),
            Self::StepOverflow => f.write_str("S4 step arithmetic overflowed"),
            Self::EmptySeedList => f.write_str("S4 seed list must not be empty"),
            Self::DuplicateSeed { seed } => write!(f, "S4 seed list repeats seed {seed}"),
            Self::OptimizerStepBudgetExceeded {
                seed,
                requested_step,
                max_step,
            } => write!(
                f,
                "S4OptimizerStepBudgetExceeded: seed {seed} requested optimizer step {requested_step} past budget {max_step}"
            ),
            Self::MissingInitialHash { field } => {
                write!(f, "S4 D9 initialization field {field} must be non-zero")
            }
        }
    }
}

impl Error for S4RunScheduleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Config(error) => Some(error),
            Self::PhaseSchedule(error) => Some(error),
            _ => None,
        }
    }
}

impl From<S4SchemaError> for S4RunScheduleError {
    fn from(error: S4SchemaError) -> Self {
        Self::Config(error)
    }
}

impl From<PhaseScheduleError> for S4RunScheduleError {
    fn from(error: PhaseScheduleError) -> Self {
        Self::PhaseSchedule(error)
    }
}
