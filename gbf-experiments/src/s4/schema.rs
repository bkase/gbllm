//! Canonical S4 notation types.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use gbf_foundation::{CanonicalJsonError, DomainHash, Hash256};
use serde::{Deserialize, Serialize};

use crate::S4_LOG_TARGET;
use crate::s4::rng::S4RngStreams;

/// D10 Gutenberg continuation optimizer-step budget.
pub const S4_OPTIMIZER_STEPS_GUTENBERG: u64 = 20_000;

/// RFC D10 marks the Gutenberg optimizer-step budget as an estimate.
pub const S4_D10_ESTIMATE_TAG: &str = "[ESTIMATE]";

/// D10 Gutenberg continuation batch size inherited from S1.
pub const S4_BATCH_SIZE: usize = 32;

/// D10 Gutenberg continuation sampled-sequence length inherited from S1.
pub const S4_SEQUENCE_LENGTH: usize = 128;

/// D10 progress-evaluation cadence for Gutenberg continuation.
pub const S4_EVAL_EVERY_STEPS: u64 = 2_000;

/// D10 progress-evaluation subset size in sampled sequences.
pub const S4_EVAL_SUBSET_SIZE: u64 = 4_096;

/// D10 AdamW learning rate for cold-optimizer Gutenberg continuation.
pub const S4_GUTENBERG_ADAMW_LR: f32 = 5.0e-4;

/// D10 AdamW beta1 value.
pub const S4_GUTENBERG_ADAMW_BETA1: f32 = 0.9;

/// D10 AdamW beta2 value.
pub const S4_GUTENBERG_ADAMW_BETA2: f32 = 0.999;

/// D10 AdamW epsilon value.
pub const S4_GUTENBERG_ADAMW_EPS: f32 = 1.0e-8;

/// D10 AdamW weight decay value.
pub const S4_GUTENBERG_ADAMW_WEIGHT_DECAY: f32 = 0.0;

/// D11 fixed seed list for closure-candidate S4 runs.
pub const S4_CANONICAL_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];

const S4_TRAIN_CONFIG_HASH_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4TrainConfig",
    "s4_train_config.v1",
    "1",
);

/// S4 hypothesis verdict status.
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

/// One of the seven F-S4 hypotheses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S4Hypothesis {
    /// H1 corpus integrity.
    #[serde(rename = "H1")]
    H1,
    /// H2 contamination and corpus-oracle validity.
    #[serde(rename = "H2")]
    H2,
    /// H3 promotion gate soundness and readiness.
    #[serde(rename = "H3")]
    H3,
    /// H4 Gutenberg quality gate.
    #[serde(rename = "H4")]
    H4,
    /// H5 oracle agreement on Gutenberg.
    #[serde(rename = "H5")]
    H5,
    /// H6 substrate and determinism.
    #[serde(rename = "H6")]
    H6,
    /// H7 reported corpus shift verdict.
    #[serde(rename = "H7")]
    H7,
}

impl S4Hypothesis {
    /// All seven S4 hypotheses in canonical closure order.
    pub const ALL: [Self; 7] = [
        Self::H1,
        Self::H2,
        Self::H3,
        Self::H4,
        Self::H5,
        Self::H6,
        Self::H7,
    ];
}

/// S4 runtime build identity.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4BuildKind {
    /// Phase-D continuation from the promoted S3 checkpoint.
    phase_d_continuation,
    /// Ablation reachability build for corpus-stack QAT isolation.
    ablation_compile_check,
    /// Test-only falsification build for broken S4 substitutes.
    s4_falsification,
}

impl S4BuildKind {
    /// All S4 build kinds in canonical matrix order.
    pub const ALL: [Self; 3] = [
        Self::phase_d_continuation,
        Self::ablation_compile_check,
        Self::s4_falsification,
    ];

    /// Stable schema/logging label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::phase_d_continuation => "phase_d_continuation",
            Self::ablation_compile_check => "ablation_compile_check",
            Self::s4_falsification => "s4_falsification",
        }
    }
}

/// AdamW parameters pinned by D10 for Gutenberg continuation.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4AdamWConfig {
    /// Learning rate.
    pub lr: f32,
    /// First-moment decay.
    pub beta1: f32,
    /// Second-moment decay.
    pub beta2: f32,
    /// Numerical epsilon.
    pub eps: f32,
    /// Weight decay.
    pub weight_decay: f32,
}

impl S4AdamWConfig {
    /// Return the RFC-pinned D10 AdamW parameters.
    #[must_use]
    pub const fn gutenberg_d10() -> Self {
        Self {
            lr: S4_GUTENBERG_ADAMW_LR,
            beta1: S4_GUTENBERG_ADAMW_BETA1,
            beta2: S4_GUTENBERG_ADAMW_BETA2,
            eps: S4_GUTENBERG_ADAMW_EPS,
            weight_decay: S4_GUTENBERG_ADAMW_WEIGHT_DECAY,
        }
    }

    fn validate_d10(&self) -> Result<(), S4SchemaError> {
        validate_finite_positive("optimizer.lr", self.lr)?;
        validate_unit_decay("optimizer.beta1", self.beta1)?;
        validate_unit_decay("optimizer.beta2", self.beta2)?;
        validate_finite_positive("optimizer.eps", self.eps)?;
        validate_finite_nonnegative("optimizer.weight_decay", self.weight_decay)?;
        require_f32_bits("optimizer.lr", self.lr, S4_GUTENBERG_ADAMW_LR)?;
        require_f32_bits("optimizer.beta1", self.beta1, S4_GUTENBERG_ADAMW_BETA1)?;
        require_f32_bits("optimizer.beta2", self.beta2, S4_GUTENBERG_ADAMW_BETA2)?;
        require_f32_bits("optimizer.eps", self.eps, S4_GUTENBERG_ADAMW_EPS)?;
        require_f32_bits(
            "optimizer.weight_decay",
            self.weight_decay,
            S4_GUTENBERG_ADAMW_WEIGHT_DECAY,
        )?;
        Ok(())
    }
}

impl Eq for S4AdamWConfig {}

/// S4 continuation phase selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4TrainPhase {
    /// Fully hardened Phase D continuation from the promoted S3 checkpoint.
    PhaseD,
}

/// Deterministic RNG implementation selected for S4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4RngKind {
    /// PCG XSL RR 128/64 MCG.
    Pcg64Mcg,
}

/// S4 device profile selector inherited from S1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4DeviceProfileKind {
    /// S1 deterministic CPU profile.
    S1CpuDeterministic,
}

/// Initial optimizer-state contract for Gutenberg continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4OptimizerStateInitial {
    /// D9 warm-weight, cold-optimizer restart.
    ZeroInitAdamW,
}

/// Initial deployed-weight source for Gutenberg continuation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S4InitialWeightSource {
    /// Promoted S3 ternary checkpoint reference `c_TS_ref`.
    #[serde(rename = "c_TS_ref")]
    CTsRef,
}

/// Initial QAT shadow-weight source for Phase-D resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S4QatShadowWeightSource {
    /// QAT shadow payload carried by the promoted S3 checkpoint reference.
    #[serde(rename = "c_TS_ref")]
    CTsRef,
}

/// Initial ternary projection rule applied at continuation entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4TernaryProjectionInitial {
    /// Phase-D hard ternary projection; no A/B/C warmup is rerun.
    PhaseDHardTernaryProjection,
}

/// RFC-pinned S4 Gutenberg continuation train configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4TrainConfig {
    /// Number of Gutenberg optimizer steps.
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
    pub optimizer: S4AdamWConfig,
    /// Phase state used for the entire continuation.
    pub phase: S4TrainPhase,
    /// Deterministic RNG kind.
    pub rng_kind: S4RngKind,
    /// Required deterministic CPU device profile.
    pub device_profile: S4DeviceProfileKind,
}

impl S4TrainConfig {
    /// Return the RFC-pinned D9/D10 Gutenberg continuation config.
    #[must_use]
    pub fn pinned() -> Self {
        Self {
            optimizer_steps: S4_OPTIMIZER_STEPS_GUTENBERG,
            batch_size: S4_BATCH_SIZE,
            sequence_length: S4_SEQUENCE_LENGTH,
            eval_every_steps: S4_EVAL_EVERY_STEPS,
            eval_subset_size: S4_EVAL_SUBSET_SIZE,
            optimizer: S4AdamWConfig::gutenberg_d10(),
            phase: S4TrainPhase::PhaseD,
            rng_kind: S4RngKind::Pcg64Mcg,
            device_profile: S4DeviceProfileKind::S1CpuDeterministic,
        }
    }

    /// Validate that the config is exactly the S4 D9/D10 closure contract.
    pub fn validate(&self) -> Result<(), S4SchemaError> {
        require_eq_u64(
            "optimizer_steps",
            self.optimizer_steps,
            S4_OPTIMIZER_STEPS_GUTENBERG,
        )?;
        require_eq_usize("batch_size", self.batch_size, S4_BATCH_SIZE)?;
        require_eq_usize("sequence_length", self.sequence_length, S4_SEQUENCE_LENGTH)?;
        require_eq_u64(
            "eval_every_steps",
            self.eval_every_steps,
            S4_EVAL_EVERY_STEPS,
        )?;
        require_eq_u64(
            "eval_subset_size",
            self.eval_subset_size,
            S4_EVAL_SUBSET_SIZE,
        )?;
        self.optimizer.validate_d10()?;
        if self.phase != S4TrainPhase::PhaseD {
            return Err(S4SchemaError::NonCanonicalTrainConfig { field: "phase" });
        }
        if self.rng_kind != S4RngKind::Pcg64Mcg {
            return Err(S4SchemaError::NonCanonicalTrainConfig { field: "rng_kind" });
        }
        if self.device_profile != S4DeviceProfileKind::S1CpuDeterministic {
            return Err(S4SchemaError::NonCanonicalTrainConfig {
                field: "device_profile",
            });
        }
        Ok(())
    }
}

impl Default for S4TrainConfig {
    fn default() -> Self {
        Self::pinned()
    }
}

/// Per-seed S4 run contract; independent of any requested seed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4SeedRunContract {
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// D10 train-config hash shared by all seeds.
    pub train_config_hash: Hash256,
    /// Seeded S4 RNG stream skeleton.
    pub rng_streams: S4RngStreams,
    /// D9 deployed-weight source.
    pub initial_weight_source: S4InitialWeightSource,
    /// D9 QAT shadow-weight source.
    pub qat_shadow_weights_initial: S4QatShadowWeightSource,
    /// D9 cold optimizer-state initialization.
    pub optimizer_state_initial: S4OptimizerStateInitial,
    /// D9 continuation phase state.
    pub phase_state_initial: S4TrainPhase,
    /// D9 Phase-D hard ternary projection at continuation entry.
    pub ternary_projection_initial: S4TernaryProjectionInitial,
    /// D9 requires InitRng to consume zero draws before the first optimizer step.
    pub init_rng_draw_count_before_first_step: u64,
    /// S4 v1 reserves ShuffleRng and consumes zero draws from it.
    pub shuffle_rng_draw_count_total: u64,
}

impl S4SeedRunContract {
    /// Construct the per-seed run contract for one canonical S4 seed.
    pub fn new(seed: u64, train_config_hash: Hash256) -> Result<Self, S4SchemaError> {
        validate_s4_seed(seed)?;
        Ok(Self {
            seed,
            train_config_hash,
            rng_streams: S4RngStreams::new(seed),
            initial_weight_source: S4InitialWeightSource::CTsRef,
            qat_shadow_weights_initial: S4QatShadowWeightSource::CTsRef,
            optimizer_state_initial: S4OptimizerStateInitial::ZeroInitAdamW,
            phase_state_initial: S4TrainPhase::PhaseD,
            ternary_projection_initial: S4TernaryProjectionInitial::PhaseDHardTernaryProjection,
            init_rng_draw_count_before_first_step: 0,
            shuffle_rng_draw_count_total: 0,
        })
    }
}

/// Validate one D11 seed value.
pub fn validate_s4_seed(seed: u64) -> Result<(), S4SchemaError> {
    if S4_CANONICAL_SEEDS.contains(&seed) {
        Ok(())
    } else {
        Err(S4SchemaError::InvalidSeed { seed })
    }
}

/// Validate the exact D11 closure-candidate seed list.
pub fn validate_s4_canonical_seed_list(seeds: &[u64]) -> Result<(), S4SchemaError> {
    if seeds == S4_CANONICAL_SEEDS {
        Ok(())
    } else {
        Err(S4SchemaError::NonCanonicalSeedList {
            observed: seeds.to_vec(),
        })
    }
}

/// Compute the canonical S4 train-config hash after exact D10 validation.
pub fn train_config_hash(config: &S4TrainConfig) -> Result<Hash256, S4SchemaError> {
    config.validate()?;
    let hash = S4_TRAIN_CONFIG_HASH_DOMAIN.hash(config)?;
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = "train_config_built",
        train_config_hash = %hash,
        optimizer_steps_gutenberg = config.optimizer_steps,
        d10_optimizer_steps_tag = S4_D10_ESTIMATE_TAG,
        phase = ?config.phase,
        "s4 train config built"
    );
    Ok(hash)
}

/// S4 report outcome tag from RFC section 11.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum S4Outcome {
    /// H1 through H6 confirmed and contamination outcome is clean.
    PassClean,
    /// H1 through H6 confirmed with a non-gating contamination warning.
    PassWithContaminationWarning,
    /// H1 corpus integrity was refuted.
    #[serde(rename = "Fail-corpus-integrity")]
    FailCorpusIntegrity,
    /// H2 contamination gate was refuted.
    #[serde(rename = "Fail-contamination")]
    FailContamination,
    /// H3 promotion gate implementation was refuted.
    #[serde(rename = "Fail-promotion-gate")]
    FailPromotionGate,
    /// H3 confirmed but the canonical S3 checkpoint was rejected.
    #[serde(rename = "Fail-promotion-gate-readiness")]
    FailPromotionGateReadiness,
    /// H4 Gutenberg quality gate was refuted.
    #[serde(rename = "Fail-quality-on-gutenberg")]
    FailQualityOnGutenberg,
    /// H5 oracle agreement was refuted.
    #[serde(rename = "Fail-oracle-disagreement")]
    FailOracleDisagreement,
    /// H6 substrate or determinism was refuted.
    #[serde(rename = "Fail-substrate")]
    FailSubstrate,
    /// Suspicious-low-bpc sentinel fired.
    #[serde(rename = "Fail-suspicious")]
    FailSuspicious,
}

impl S4Outcome {
    /// All S4 report outcome tags currently accepted by the schema.
    pub const ALL: [Self; 10] = [
        Self::PassClean,
        Self::PassWithContaminationWarning,
        Self::FailCorpusIntegrity,
        Self::FailContamination,
        Self::FailPromotionGate,
        Self::FailPromotionGateReadiness,
        Self::FailQualityOnGutenberg,
        Self::FailOracleDisagreement,
        Self::FailSubstrate,
        Self::FailSuspicious,
    ];
}

/// S4 decision tag from RFC section 11.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum S4Decision {
    /// Proceed to S5 without contamination warning.
    #[serde(rename = "ProceedToS5")]
    ProceedToS5,
    /// Proceed to S5 while carrying an explicit contamination warning.
    #[serde(rename = "ProceedToS5-with-contamination-warning")]
    ProceedToS5WithContaminationWarning,
    /// Investigation is required before S4 can close.
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

/// Completion state recorded for each Gutenberg seed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum S4Completion {
    /// The run completed its requested optimizer steps.
    Completed,
    /// The run observed divergence at the recorded train step.
    DivergedAt {
        /// First diverged global train step.
        step: u64,
    },
    /// The run product was not reached because an earlier gate stopped.
    NotReached,
}

/// Verifier and early-gate inputs consumed by the future S4 outcome dispatcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4VerifierBundle {
    /// Corpus integrity gate.
    pub corpus_integrity_passed: bool,
    /// Cross-corpus contamination gate.
    pub contamination_passed: bool,
    /// Promotion gate implementation soundness.
    pub promotion_gate_sound: bool,
    /// Whether the canonical S3 checkpoint was accepted for promotion.
    pub promotion_gate_accepted_canonical: bool,
    /// Gutenberg quality gate.
    pub gutenberg_quality_passed: bool,
    /// Oracle agreement on Gutenberg.
    pub oracle_agreement_passed: bool,
    /// S4 substrate and determinism gate.
    pub substrate_passed: bool,
    /// Suspicious-low median bpc_char sentinel.
    pub suspicious_low_bpc: bool,
    /// Whether contamination produced a non-gating warning.
    pub contamination_warning: bool,
    /// Per-seed completion states across the five S4 seeds.
    pub completions: Vec<S4Completion>,
    /// Explicit verdict status for all seven hypotheses.
    pub hypothesis_statuses: BTreeMap<S4Hypothesis, HypothesisStatus>,
}

impl S4VerifierBundle {
    /// Construct a closure-candidate bundle with all gates and hypotheses passing.
    #[must_use]
    pub fn closure_candidate() -> Self {
        Self {
            corpus_integrity_passed: true,
            contamination_passed: true,
            promotion_gate_sound: true,
            promotion_gate_accepted_canonical: true,
            gutenberg_quality_passed: true,
            oracle_agreement_passed: true,
            substrate_passed: true,
            suspicious_low_bpc: false,
            contamination_warning: false,
            completions: vec![S4Completion::Completed; S4_CANONICAL_SEEDS.len()],
            hypothesis_statuses: all_confirmed_hypotheses(),
        }
    }

    /// Return the status for one S4 hypothesis.
    #[must_use]
    pub fn status(&self, hypothesis: S4Hypothesis) -> HypothesisStatus {
        self.hypothesis_statuses
            .get(&hypothesis)
            .cloned()
            .unwrap_or_else(|| HypothesisStatus::NotEvaluatedDueToPriorGate {
                reason: "missing hypothesis status".to_owned(),
            })
    }
}

fn all_confirmed_hypotheses() -> BTreeMap<S4Hypothesis, HypothesisStatus> {
    S4Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| (hypothesis, HypothesisStatus::Confirmed))
        .collect()
}

fn validate_finite(field: &'static str, value: f32) -> Result<(), S4SchemaError> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(S4SchemaError::NonFiniteF32 { field, value })
    }
}

fn validate_finite_positive(field: &'static str, value: f32) -> Result<(), S4SchemaError> {
    validate_finite(field, value)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(S4SchemaError::NonPositiveF32 { field, value })
    }
}

fn validate_finite_nonnegative(field: &'static str, value: f32) -> Result<(), S4SchemaError> {
    validate_finite(field, value)?;
    if value >= 0.0 {
        Ok(())
    } else {
        Err(S4SchemaError::NegativeF32 { field, value })
    }
}

fn validate_unit_decay(field: &'static str, value: f32) -> Result<(), S4SchemaError> {
    validate_finite(field, value)?;
    if (0.0..1.0).contains(&value) {
        Ok(())
    } else {
        Err(S4SchemaError::InvalidAdamWDecay { field, value })
    }
}

fn require_eq_u64(field: &'static str, observed: u64, expected: u64) -> Result<(), S4SchemaError> {
    if observed == expected {
        Ok(())
    } else {
        Err(S4SchemaError::NonCanonicalTrainConfig { field })
    }
}

fn require_eq_usize(
    field: &'static str,
    observed: usize,
    expected: usize,
) -> Result<(), S4SchemaError> {
    if observed == expected {
        Ok(())
    } else {
        Err(S4SchemaError::NonCanonicalTrainConfig { field })
    }
}

fn require_f32_bits(
    field: &'static str,
    observed: f32,
    expected: f32,
) -> Result<(), S4SchemaError> {
    if observed.to_bits() == expected.to_bits() {
        Ok(())
    } else {
        Err(S4SchemaError::NonCanonicalTrainConfig { field })
    }
}

/// Errors raised by S4 schema/config contract helpers.
#[derive(Debug, PartialEq)]
pub enum S4SchemaError {
    /// A train-config field is valid in isolation but not the pinned D10 value.
    NonCanonicalTrainConfig {
        /// Rejected field.
        field: &'static str,
    },
    /// A floating-point value must be finite.
    NonFiniteF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// A floating-point value must be positive.
    NonPositiveF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// A floating-point value must be non-negative.
    NegativeF32 {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// AdamW decay parameters must be finite and in [0, 1).
    InvalidAdamWDecay {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
    /// Seed is not one of D11's fixed values.
    InvalidSeed {
        /// Rejected seed.
        seed: u64,
    },
    /// Closure-candidate runs must use exactly the D11 seed list.
    NonCanonicalSeedList {
        /// Rejected seed list.
        observed: Vec<u64>,
    },
    /// Canonical JSON hashing failed.
    CanonicalJson {
        /// Error message.
        message: String,
    },
}

impl fmt::Display for S4SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonCanonicalTrainConfig { field } => {
                write!(
                    f,
                    "S4 train config field {field} is not the D9/D10 pinned value"
                )
            }
            Self::NonFiniteF32 { field, value } => {
                write!(f, "S4 field {field} must be finite, got {value}")
            }
            Self::NonPositiveF32 { field, value } => {
                write!(f, "S4 field {field} must be positive, got {value}")
            }
            Self::NegativeF32 { field, value } => {
                write!(f, "S4 field {field} must be non-negative, got {value}")
            }
            Self::InvalidAdamWDecay { field, value } => {
                write!(f, "S4 AdamW decay {field} must be in [0, 1), got {value}")
            }
            Self::InvalidSeed { seed } => {
                write!(f, "S4 seed {seed} is not in the D11 fixed seed list")
            }
            Self::NonCanonicalSeedList { observed } => {
                write!(
                    f,
                    "S4 seed list {observed:?} is not the D11 fixed seed list"
                )
            }
            Self::CanonicalJson { message } => write!(f, "{message}"),
        }
    }
}

impl Error for S4SchemaError {}

impl From<CanonicalJsonError> for S4SchemaError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson {
            message: error.to_string(),
        }
    }
}
