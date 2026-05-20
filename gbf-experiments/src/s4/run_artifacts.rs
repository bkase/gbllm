//! S4 Gutenberg run-log, checkpoint, and FP-reference artifact surface.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256, SemVer};
use serde::{Deserialize, Serialize};

use crate::S4_LOG_TARGET;
use crate::s4::run::{S4RunScheduleError, progress_eval_steps};
use crate::s4::schema::{
    S4_OPTIMIZER_STEPS_GUTENBERG, S4BuildKind, S4Completion, S4InitialWeightSource, S4Outcome,
    S4SchemaError, S4TrainConfig, validate_s4_seed,
};

/// Schema id for completed Gutenberg continuation run logs.
pub const S4_GUTENBERG_RUN_LOG_SCHEMA: &str = "s4_gutenberg_run_log.v1";

/// Schema id for completed Gutenberg continuation checkpoint sidecars.
pub const S4_GUTENBERG_CHECKPOINT_SCHEMA: &str = "s4_gutenberg_checkpoint.v1";

/// Schema id for full-precision/QAT-shadow reference artifacts.
pub const S4_FP_REFERENCE_SCHEMA: &str = "s4_fp_reference.v1";

/// RFC-pinned S4 FP reference kind.
pub const S4_FP_REFERENCE_KIND_QAT_SHADOW_AFTER_GUTENBERG: &str =
    "qat_shadow_weights_after_gutenberg_continuation";

const PRODUCT_SCHEMA_VERSION: &str = "1";
const RUN_LOG_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4GutenbergRunLog",
    S4_GUTENBERG_RUN_LOG_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const CHECKPOINT_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4GutenbergCheckpoint",
    S4_GUTENBERG_CHECKPOINT_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);
const FP_REFERENCE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4FpReference",
    S4_FP_REFERENCE_SCHEMA,
    PRODUCT_SCHEMA_VERSION,
);

/// Summary of final gradient norms for `s4_gutenberg_run_log.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4GradNormSummary {
    /// Final global L2 norm over all trainable gradients.
    pub global_l2: f64,
    /// Largest per-tensor L2 norm observed in the final step.
    pub max_l2: f64,
    /// Mean per-tensor L2 norm observed in the final step.
    pub mean_l2: f64,
}

/// Completed S4 Gutenberg continuation run log.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4GutenbergRunLog {
    /// Schema id, always `s4_gutenberg_run_log.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash inherited from c_TS lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash for the continuation corpus.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// D10 train configuration hash.
    pub train_config_hash: Hash256,
    /// Promotion-gate artifact self-hash that accepted c_TS.
    pub promotion_gate_self_hash: Hash256,
    /// Promoted S3 checkpoint self-hash used as c_TS.
    #[serde(rename = "c_TS_checkpoint_self_hash")]
    pub c_ts_checkpoint_self_hash: Hash256,
    /// SHA-256 of deployed ternary tensors at continuation entry.
    pub initial_checkpoint_payload_sha: Hash256,
    /// Initial deployed-weight source, always `c_TS_ref`.
    pub initial_weight_source: S4InitialWeightSource,
    /// SHA-256 of FP/QAT shadow tensors at continuation entry.
    pub initial_fp_shadow_payload_sha: Hash256,
    /// InitRng draw count before optimizer step 1; D9 requires zero.
    pub init_rng_draw_count_before_first_step: u64,
    /// ShuffleRng draw count for S4 v1; D9/D10 require zero.
    pub shuffle_rng_draw_count_total: u64,
    /// One finite train loss for each optimizer step, steps 1..=20,000.
    pub losses: Vec<(u64, f64)>,
    /// Reset-context progress eval BPC values, including step 0.
    pub eval_points: Vec<(u64, f64)>,
    /// Final finite gradient norm summary.
    pub final_grad_norms: S4GradNormSummary,
    /// Self-hash over canonical JSON with this field omitted.
    pub run_log_self_hash: Hash256,
}

impl S4GutenbergRunLog {
    /// Return a copy with `run_log_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S4RunArtifactError> {
        self.run_log_self_hash = Hash256::ZERO;
        self.validate_structure_without_self_hash()?;
        self.run_log_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes including `run_log_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4RunArtifactError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4RunArtifactError::CanonicalJson)
    }

    /// Compute the run-log self-hash with `run_log_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        compute_self_hash(self, "run_log_self_hash", RUN_LOG_DOMAIN)
    }

    /// Validate structure and self-hash before writing the artifact.
    pub fn validate_canonical_write(&self) -> Result<(), S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        let expected = self.compute_self_hash()?;
        if expected != self.run_log_self_hash {
            return Err(S4RunArtifactError::SelfHashMismatch {
                field: "run_log_self_hash",
                expected,
                observed: self.run_log_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure_without_self_hash(&self) -> Result<(), S4RunArtifactError> {
        validate_schema(
            "schema",
            &self.schema,
            S4_GUTENBERG_RUN_LOG_SCHEMA,
            "s4_gutenberg_run_log.v1",
        )?;
        validate_s4_seed(self.seed).map_err(S4RunArtifactError::Schema)?;
        if self.initial_weight_source != S4InitialWeightSource::CTsRef {
            return Err(S4RunArtifactError::InvalidField {
                field: "initial_weight_source",
            });
        }
        if self.init_rng_draw_count_before_first_step != 0 {
            return Err(S4RunArtifactError::InvalidField {
                field: "init_rng_draw_count_before_first_step",
            });
        }
        if self.shuffle_rng_draw_count_total != 0 {
            return Err(S4RunArtifactError::InvalidField {
                field: "shuffle_rng_draw_count_total",
            });
        }
        validate_completed_losses(&self.losses)?;
        validate_completed_eval_points(&self.eval_points)?;
        self.final_grad_norms.validate()?;
        Ok(())
    }
}

impl S4GradNormSummary {
    fn validate(&self) -> Result<(), S4RunArtifactError> {
        validate_finite_nonnegative_metric("final_grad_norms.global_l2", None, self.global_l2)?;
        validate_finite_nonnegative_metric("final_grad_norms.max_l2", None, self.max_l2)?;
        validate_finite_nonnegative_metric("final_grad_norms.mean_l2", None, self.mean_l2)?;
        Ok(())
    }
}

/// Completed S4 Gutenberg checkpoint metadata sidecar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4GutenbergCheckpointMetadata {
    /// Schema id, always `s4_gutenberg_checkpoint.v1`.
    pub schema: String,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Promoted S3 checkpoint self-hash used as c_TS.
    #[serde(rename = "c_TS_checkpoint_self_hash")]
    pub c_ts_checkpoint_self_hash: Hash256,
    /// Promotion-gate artifact self-hash that accepted c_TS.
    pub promotion_gate_self_hash: Hash256,
    /// SHA-256 of deployed ternary tensor payload bytes.
    pub deployed_tensor_payload_sha: Hash256,
    /// SHA-256 of FP/QAT shadow tensor payload bytes.
    pub fp_shadow_tensor_payload_sha: Hash256,
    /// SHA-256 of the Gutenberg train split.
    pub corpus_train_sha: Hash256,
    /// SHA-256 of the Gutenberg validation split.
    pub corpus_val_sha: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// TinyStories manifest self-hash inherited from c_TS lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Model configuration hash.
    pub model_config_hash: Hash256,
    /// D10 train configuration hash.
    pub train_config_hash: Hash256,
    /// Build kind, always `phase_d_continuation`.
    pub build_kind: S4BuildKind,
    /// Build configuration hash.
    pub build_config_hash: Hash256,
    /// Dependency lockfile hash.
    pub dependency_lockfile_sha: Hash256,
    /// Rust toolchain hash.
    pub rust_toolchain_hash: Hash256,
    /// Deterministic device profile hash.
    pub device_profile_hash: Hash256,
    /// Pass implementation version.
    pub pass_version: SemVer,
    /// Final optimizer step, always 20,000 for a completed checkpoint.
    pub final_step: u64,
    /// Final finite train loss in nats per token.
    pub final_train_loss: f64,
    /// Completion state, always `Completed` for emitted checkpoints.
    pub completion: S4Completion,
    /// Self-hash over canonical JSON with this field omitted.
    pub checkpoint_self_hash: Hash256,
}

impl S4GutenbergCheckpointMetadata {
    /// Return a copy with `checkpoint_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S4RunArtifactError> {
        self.checkpoint_self_hash = Hash256::ZERO;
        self.validate_structure_without_self_hash()?;
        self.checkpoint_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes including `checkpoint_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4RunArtifactError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4RunArtifactError::CanonicalJson)
    }

    /// Compute the checkpoint self-hash with `checkpoint_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        compute_self_hash(self, "checkpoint_self_hash", CHECKPOINT_DOMAIN)
    }

    /// Validate structure and self-hash before writing the artifact.
    pub fn validate_canonical_write(&self) -> Result<(), S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        let expected = self.compute_self_hash()?;
        if expected != self.checkpoint_self_hash {
            return Err(S4RunArtifactError::SelfHashMismatch {
                field: "checkpoint_self_hash",
                expected,
                observed: self.checkpoint_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure_without_self_hash(&self) -> Result<(), S4RunArtifactError> {
        validate_schema(
            "schema",
            &self.schema,
            S4_GUTENBERG_CHECKPOINT_SCHEMA,
            "s4_gutenberg_checkpoint.v1",
        )?;
        validate_s4_seed(self.seed).map_err(S4RunArtifactError::Schema)?;
        if self.build_kind != S4BuildKind::phase_d_continuation {
            return Err(S4RunArtifactError::InvalidField {
                field: "build_kind",
            });
        }
        if self.final_step != S4_OPTIMIZER_STEPS_GUTENBERG {
            return Err(S4RunArtifactError::InvalidFinalStep {
                expected: S4_OPTIMIZER_STEPS_GUTENBERG,
                observed: self.final_step,
            });
        }
        validate_finite_nonnegative_metric(
            "final_train_loss",
            Some(self.final_step),
            self.final_train_loss,
        )?;
        if self.completion != S4Completion::Completed {
            return Err(S4RunArtifactError::DivergedRunCannotCheckpoint {
                observed: self.completion.clone(),
            });
        }
        Ok(())
    }
}

/// Full-precision/QAT-shadow reference produced after Gutenberg continuation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4FpReferenceArtifact {
    /// Schema id, always `s4_fp_reference.v1`.
    pub schema: String,
    /// Gutenberg continuation seed.
    pub seed: u64,
    /// Source `s4_gutenberg_checkpoint.v1` self-hash.
    pub source_checkpoint_self_hash: Hash256,
    /// FP reference kind, always QAT shadow weights after Gutenberg continuation.
    pub fp_reference_kind: String,
    /// SHA-256 of the full-precision/QAT-shadow tensor payload.
    pub fp_shadow_payload_sha: Hash256,
    /// TinyStories manifest self-hash inherited from c_TS lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation split.
    pub corpus_val_sha: Hash256,
    /// Self-hash over canonical JSON with this field omitted.
    pub fp_reference_self_hash: Hash256,
}

impl S4FpReferenceArtifact {
    /// Return a copy with `fp_reference_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, S4RunArtifactError> {
        self.fp_reference_self_hash = Hash256::ZERO;
        self.validate_structure_without_self_hash()?;
        self.fp_reference_self_hash = self.compute_self_hash()?;
        Ok(self)
    }

    /// Canonical JSON bytes including `fp_reference_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4RunArtifactError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4RunArtifactError::CanonicalJson)
    }

    /// Compute the FP-reference self-hash with `fp_reference_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        compute_self_hash(self, "fp_reference_self_hash", FP_REFERENCE_DOMAIN)
    }

    /// Validate structure and self-hash before writing the artifact.
    pub fn validate_canonical_write(&self) -> Result<(), S4RunArtifactError> {
        self.validate_structure_without_self_hash()?;
        let expected = self.compute_self_hash()?;
        if expected != self.fp_reference_self_hash {
            return Err(S4RunArtifactError::SelfHashMismatch {
                field: "fp_reference_self_hash",
                expected,
                observed: self.fp_reference_self_hash,
            });
        }
        Ok(())
    }

    /// Validate lineage against the checkpoint metadata that produced this FP reference.
    pub fn validate_against_checkpoint(
        &self,
        checkpoint: &S4GutenbergCheckpointMetadata,
    ) -> Result<(), S4RunArtifactError> {
        self.validate_canonical_write()?;
        checkpoint.validate_canonical_write()?;
        validate_lineage_hash(
            "source_checkpoint_self_hash",
            checkpoint.checkpoint_self_hash,
            self.source_checkpoint_self_hash,
        )?;
        validate_lineage_hash(
            "fp_shadow_payload_sha",
            checkpoint.fp_shadow_tensor_payload_sha,
            self.fp_shadow_payload_sha,
        )?;
        validate_lineage_hash(
            "tinystories_manifest_self_hash",
            checkpoint.tinystories_manifest_self_hash,
            self.tinystories_manifest_self_hash,
        )?;
        validate_lineage_hash(
            "gutenberg_manifest_self_hash",
            checkpoint.gutenberg_manifest_self_hash,
            self.gutenberg_manifest_self_hash,
        )?;
        validate_lineage_hash(
            "corpus_val_sha",
            checkpoint.corpus_val_sha,
            self.corpus_val_sha,
        )?;
        Ok(())
    }

    /// Validate lineage against the checkpoint and Gutenberg manifest validation split hash.
    pub fn validate_against_checkpoint_and_gutenberg_val_sha(
        &self,
        checkpoint: &S4GutenbergCheckpointMetadata,
        gutenberg_manifest_val_sha: Hash256,
    ) -> Result<(), S4RunArtifactError> {
        self.validate_against_checkpoint(checkpoint)?;
        validate_lineage_hash(
            "corpus_val_sha",
            gutenberg_manifest_val_sha,
            self.corpus_val_sha,
        )
    }

    fn validate_structure_without_self_hash(&self) -> Result<(), S4RunArtifactError> {
        validate_schema(
            "schema",
            &self.schema,
            S4_FP_REFERENCE_SCHEMA,
            "s4_fp_reference.v1",
        )?;
        validate_s4_seed(self.seed).map_err(S4RunArtifactError::Schema)?;
        if self.fp_reference_kind != S4_FP_REFERENCE_KIND_QAT_SHADOW_AFTER_GUTENBERG {
            return Err(S4RunArtifactError::InvalidField {
                field: "fp_reference_kind",
            });
        }
        Ok(())
    }
}

/// Per-step diagnostics consumed by the D13 fail-closed divergence detector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct S4StepDiagnostics {
    /// Optimizer step for the diagnostics row.
    pub step: u64,
    /// Train loss in nats per token for this step.
    pub loss_nats_per_token: f64,
    /// Optional moving-average loss used by the finite surprise detector.
    pub moving_average_loss_nats_per_token: Option<f64>,
    /// Global gradient L2 norm for this step.
    pub grad_global_l2: f64,
}

/// Config for finite-loss spike surprise detection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct S4LossSpikeSurpriseConfig {
    /// One-sided loss-above-moving-average delta threshold.
    pub threshold_nats_per_token: f64,
    /// Number of consecutive threshold breaches required to emit the surprise.
    pub consecutive_steps: u64,
}

/// D13 divergence observation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4DivergenceObserved {
    /// First non-finite training loss.
    NonFiniteLoss,
    /// First non-finite gradient norm.
    NonFiniteGradNorm,
}

/// D13 divergence event that never serializes NaN or infinity payloads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4DivergenceEvent {
    /// First optimizer step where D13 divergence was observed.
    pub step: u64,
    /// Divergence kind.
    pub observed: S4DivergenceObserved,
    /// Last finite loss before or at the divergence point, if available.
    pub last_finite_loss: Option<f64>,
}

impl S4DivergenceEvent {
    /// Validate that the event itself is safe to serialize.
    pub fn validate(&self) -> Result<(), S4RunArtifactError> {
        if self.step == 0 {
            return Err(S4RunArtifactError::UnexpectedTrainStep {
                index: 0,
                expected: 1,
                observed: 0,
            });
        }
        if let Some(loss) = self.last_finite_loss {
            validate_finite_nonnegative_metric(
                "divergence_event.last_finite_loss",
                Some(self.step),
                loss,
            )?;
        }
        Ok(())
    }
}

/// Return the first D13 divergence event, if any, from monotonically ordered step diagnostics.
pub fn first_d13_divergence_event(
    diagnostics: &[S4StepDiagnostics],
) -> Result<Option<S4DivergenceEvent>, S4RunArtifactError> {
    let mut last_finite_loss = None;
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let expected_step = index as u64 + 1;
        if diagnostic.step != expected_step {
            return Err(S4RunArtifactError::UnexpectedTrainStep {
                index,
                expected: expected_step,
                observed: diagnostic.step,
            });
        }
        if !diagnostic.loss_nats_per_token.is_finite() {
            let event = S4DivergenceEvent {
                step: diagnostic.step,
                observed: S4DivergenceObserved::NonFiniteLoss,
                last_finite_loss,
            };
            event.validate()?;
            return Ok(Some(event));
        }
        if diagnostic.loss_nats_per_token < 0.0 {
            return Err(S4RunArtifactError::NegativeMetric {
                field: "loss_nats_per_token",
                step: Some(diagnostic.step),
                value: diagnostic.loss_nats_per_token,
            });
        }
        if !diagnostic.grad_global_l2.is_finite() {
            let event = S4DivergenceEvent {
                step: diagnostic.step,
                observed: S4DivergenceObserved::NonFiniteGradNorm,
                last_finite_loss: Some(diagnostic.loss_nats_per_token),
            };
            event.validate()?;
            return Ok(Some(event));
        }
        if diagnostic.grad_global_l2 < 0.0 {
            return Err(S4RunArtifactError::NegativeMetric {
                field: "grad_global_l2",
                step: Some(diagnostic.step),
                value: diagnostic.grad_global_l2,
            });
        }
        last_finite_loss = Some(diagnostic.loss_nats_per_token);
    }
    Ok(None)
}

/// Non-gating finite-loss surprise observation kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum S4RunSurpriseObserved {
    /// Finite loss rose above its moving average for the configured window.
    FiniteLossSpikeSurprise,
}

/// Non-gating surprise event for finite loss spikes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4RunSurpriseEvent {
    /// Optimizer step that completed the surprise window.
    pub step: u64,
    /// Surprise kind.
    pub observed: S4RunSurpriseObserved,
    /// Finite loss at the surprise step.
    pub loss_nats_per_token: f64,
    /// Finite moving-average loss at the surprise step.
    pub moving_average_loss_nats_per_token: f64,
    /// One-sided loss-above-moving-average threshold.
    pub threshold_nats_per_token: f64,
    /// Consecutive threshold breaches required for this surprise.
    pub consecutive_steps: u64,
}

impl S4RunSurpriseEvent {
    /// Validate that the surprise event is safe to serialize.
    pub fn validate(&self) -> Result<(), S4RunArtifactError> {
        if self.step == 0 {
            return Err(S4RunArtifactError::UnexpectedTrainStep {
                index: 0,
                expected: 1,
                observed: 0,
            });
        }
        validate_finite_nonnegative_metric(
            "surprise_event.loss_nats_per_token",
            Some(self.step),
            self.loss_nats_per_token,
        )?;
        validate_finite_nonnegative_metric(
            "surprise_event.moving_average_loss_nats_per_token",
            Some(self.step),
            self.moving_average_loss_nats_per_token,
        )?;
        validate_finite_positive_metric(
            "surprise_event.threshold_nats_per_token",
            Some(self.step),
            self.threshold_nats_per_token,
        )?;
        if self.consecutive_steps == 0 {
            return Err(S4RunArtifactError::InvalidField {
                field: "surprise_event.consecutive_steps",
            });
        }
        Ok(())
    }
}

/// Return the first non-gating finite-loss spike surprise event, if any.
pub fn first_loss_spike_surprise_event(
    diagnostics: &[S4StepDiagnostics],
    config: S4LossSpikeSurpriseConfig,
) -> Result<Option<S4RunSurpriseEvent>, S4RunArtifactError> {
    config.validate()?;
    let mut threshold_streak = 0_u64;
    for (index, diagnostic) in diagnostics.iter().enumerate() {
        let expected_step = index as u64 + 1;
        if diagnostic.step != expected_step {
            return Err(S4RunArtifactError::UnexpectedTrainStep {
                index,
                expected: expected_step,
                observed: diagnostic.step,
            });
        }
        validate_finite_nonnegative_metric(
            "loss_nats_per_token",
            Some(diagnostic.step),
            diagnostic.loss_nats_per_token,
        )?;
        let Some(moving_average_loss) = diagnostic.moving_average_loss_nats_per_token else {
            threshold_streak = 0;
            continue;
        };
        validate_finite_nonnegative_metric(
            "moving_average_loss_nats_per_token",
            Some(diagnostic.step),
            moving_average_loss,
        )?;
        if diagnostic.loss_nats_per_token - moving_average_loss > config.threshold_nats_per_token {
            threshold_streak += 1;
        } else {
            threshold_streak = 0;
        }
        if threshold_streak >= config.consecutive_steps {
            let event = S4RunSurpriseEvent {
                step: diagnostic.step,
                observed: S4RunSurpriseObserved::FiniteLossSpikeSurprise,
                loss_nats_per_token: diagnostic.loss_nats_per_token,
                moving_average_loss_nats_per_token: moving_average_loss,
                threshold_nats_per_token: config.threshold_nats_per_token,
                consecutive_steps: config.consecutive_steps,
            };
            event.validate()?;
            return Ok(Some(event));
        }
    }
    Ok(None)
}

impl S4LossSpikeSurpriseConfig {
    fn validate(&self) -> Result<(), S4RunArtifactError> {
        validate_finite_positive_metric(
            "loss_spike_surprise.threshold_nats_per_token",
            None,
            self.threshold_nats_per_token,
        )?;
        if self.consecutive_steps == 0 {
            return Err(S4RunArtifactError::InvalidField {
                field: "loss_spike_surprise.consecutive_steps",
            });
        }
        Ok(())
    }
}

/// D13 fail-closed mapping: any seed divergence forces `Fail-substrate`.
#[must_use]
pub fn d13_fail_closed_outcome(completions: &[S4Completion]) -> Option<S4Outcome> {
    completions
        .iter()
        .any(|completion| matches!(completion, S4Completion::DivergedAt { .. }))
        .then_some(S4Outcome::FailSubstrate)
}

/// Write a completed run log as canonical JSON.
pub fn write_s4_gutenberg_run_log(
    path: &Path,
    run_log: &S4GutenbergRunLog,
) -> Result<(), S4RunArtifactError> {
    let bytes = run_log.canonical_bytes()?;
    write_artifact_bytes(path, &bytes)?;
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = "s4::run_artifacts::run_log_emitted",
        schema = S4_GUTENBERG_RUN_LOG_SCHEMA,
        seed = run_log.seed,
        run_log_self_hash = %run_log.run_log_self_hash,
        path = %path.display(),
        "s4 gutenberg run log emitted"
    );
    Ok(())
}

/// Write completed checkpoint metadata as canonical JSON.
pub fn write_s4_gutenberg_checkpoint_metadata(
    path: &Path,
    checkpoint: &S4GutenbergCheckpointMetadata,
) -> Result<(), S4RunArtifactError> {
    let bytes = checkpoint.canonical_bytes()?;
    write_artifact_bytes(path, &bytes)?;
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = "s4::run_artifacts::checkpoint_emitted",
        schema = S4_GUTENBERG_CHECKPOINT_SCHEMA,
        seed = checkpoint.seed,
        checkpoint_self_hash = %checkpoint.checkpoint_self_hash,
        path = %path.display(),
        "s4 gutenberg checkpoint metadata emitted"
    );
    Ok(())
}

/// Write the S4 FP reference artifact as canonical JSON.
pub fn write_s4_fp_reference(
    path: &Path,
    fp_reference: &S4FpReferenceArtifact,
) -> Result<(), S4RunArtifactError> {
    let bytes = fp_reference.canonical_bytes()?;
    write_artifact_bytes(path, &bytes)?;
    tracing::info!(
        target: S4_LOG_TARGET,
        event_name = "s4::run_artifacts::fp_reference_emitted",
        schema = S4_FP_REFERENCE_SCHEMA,
        seed = fp_reference.seed,
        fp_reference_self_hash = %fp_reference.fp_reference_self_hash,
        path = %path.display(),
        "s4 fp reference emitted"
    );
    Ok(())
}

fn validate_completed_losses(losses: &[(u64, f64)]) -> Result<(), S4RunArtifactError> {
    let expected_count = S4_OPTIMIZER_STEPS_GUTENBERG as usize;
    if losses.len() != expected_count {
        return Err(S4RunArtifactError::LossCount {
            expected: expected_count,
            observed: losses.len(),
        });
    }
    for (index, (step, loss)) in losses.iter().copied().enumerate() {
        let expected_step = index as u64 + 1;
        if step != expected_step {
            return Err(S4RunArtifactError::UnexpectedTrainStep {
                index,
                expected: expected_step,
                observed: step,
            });
        }
        validate_finite_nonnegative_metric("losses.loss_nats_per_token", Some(step), loss)?;
    }
    Ok(())
}

fn validate_completed_eval_points(eval_points: &[(u64, f64)]) -> Result<(), S4RunArtifactError> {
    let expected_steps =
        progress_eval_steps(&S4TrainConfig::pinned()).map_err(S4RunArtifactError::RunSchedule)?;
    if eval_points.len() != expected_steps.len() {
        return Err(S4RunArtifactError::EvalPointCount {
            expected: expected_steps.len(),
            observed: eval_points.len(),
        });
    }
    for (index, ((step, bpc), expected_step)) in eval_points
        .iter()
        .copied()
        .zip(expected_steps.iter().copied())
        .enumerate()
    {
        if step != expected_step {
            return Err(S4RunArtifactError::UnexpectedEvalStep {
                index,
                expected: expected_step,
                observed: step,
            });
        }
        validate_finite_nonnegative_metric("eval_points.bpc", Some(step), bpc)?;
    }
    Ok(())
}

fn validate_schema(
    field: &'static str,
    observed: &str,
    expected: &'static str,
    artifact: &'static str,
) -> Result<(), S4RunArtifactError> {
    if observed == expected {
        Ok(())
    } else {
        Err(S4RunArtifactError::InvalidSchema {
            field,
            artifact,
            expected,
            observed: observed.to_owned(),
        })
    }
}

fn validate_lineage_hash(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S4RunArtifactError> {
    if expected == observed {
        Ok(())
    } else {
        Err(S4RunArtifactError::LineageMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn validate_finite_nonnegative_metric(
    field: &'static str,
    step: Option<u64>,
    value: f64,
) -> Result<(), S4RunArtifactError> {
    if !value.is_finite() {
        return Err(S4RunArtifactError::NonFiniteMetric { field, step, value });
    }
    if value < 0.0 {
        return Err(S4RunArtifactError::NegativeMetric { field, step, value });
    }
    Ok(())
}

fn validate_finite_positive_metric(
    field: &'static str,
    step: Option<u64>,
    value: f64,
) -> Result<(), S4RunArtifactError> {
    validate_finite_nonnegative_metric(field, step, value)?;
    if value == 0.0 {
        return Err(S4RunArtifactError::InvalidField { field });
    }
    Ok(())
}

fn write_artifact_bytes(path: &Path, bytes: &[u8]) -> Result<(), S4RunArtifactError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(S4RunArtifactError::Io)?;
    }
    fs::write(path, bytes).map_err(S4RunArtifactError::Io)
}

fn compute_self_hash<T: Serialize>(
    payload: &T,
    self_hash_field: &'static str,
    domain: DomainHash<'static>,
) -> Result<Hash256, S4RunArtifactError> {
    let mut value = serde_json::to_value(payload).map_err(S4RunArtifactError::Json)?;
    value
        .as_object_mut()
        .ok_or(S4RunArtifactError::ExpectedObjectForSelfHash)?
        .remove(self_hash_field);
    let canonical =
        CanonicalJson::value_to_vec(&value).map_err(S4RunArtifactError::CanonicalJson)?;
    domain
        .hash_canonical_bytes(&canonical)
        .map_err(S4RunArtifactError::CanonicalJson)
}

/// Errors from S4 run artifact validation and emission.
#[derive(Debug)]
pub enum S4RunArtifactError {
    /// Schema field did not match the artifact contract.
    InvalidSchema {
        /// Field name.
        field: &'static str,
        /// Artifact schema being validated.
        artifact: &'static str,
        /// Expected schema id.
        expected: &'static str,
        /// Observed schema id.
        observed: String,
    },
    /// A constrained field drifted away from the RFC pin.
    InvalidField {
        /// Invalid field name.
        field: &'static str,
    },
    /// A completed run log had the wrong train-loss count.
    LossCount {
        /// Expected loss row count.
        expected: usize,
        /// Observed loss row count.
        observed: usize,
    },
    /// A completed run log had the wrong eval-point count.
    EvalPointCount {
        /// Expected eval row count.
        expected: usize,
        /// Observed eval row count.
        observed: usize,
    },
    /// A train step did not match the canonical 1-based sequence.
    UnexpectedTrainStep {
        /// Zero-based row index.
        index: usize,
        /// Expected step.
        expected: u64,
        /// Observed step.
        observed: u64,
    },
    /// An eval step did not match the canonical progress-eval cadence.
    UnexpectedEvalStep {
        /// Zero-based row index.
        index: usize,
        /// Expected step.
        expected: u64,
        /// Observed step.
        observed: u64,
    },
    /// A metric was NaN or infinite and must not be serialized.
    NonFiniteMetric {
        /// Field name.
        field: &'static str,
        /// Associated optimizer/eval step, when available.
        step: Option<u64>,
        /// Observed invalid value.
        value: f64,
    },
    /// A metric was finite but negative.
    NegativeMetric {
        /// Field name.
        field: &'static str,
        /// Associated optimizer/eval step, when available.
        step: Option<u64>,
        /// Observed invalid value.
        value: f64,
    },
    /// Completed checkpoint metadata had a non-final step.
    InvalidFinalStep {
        /// Expected final step.
        expected: u64,
        /// Observed final step.
        observed: u64,
    },
    /// Diverged or unreached runs cannot emit completed checkpoint metadata.
    DivergedRunCannotCheckpoint {
        /// Observed completion value.
        observed: S4Completion,
    },
    /// Stored self-hash differed from recomputation.
    SelfHashMismatch {
        /// Self-hash field name.
        field: &'static str,
        /// Expected recomputed self-hash.
        expected: Hash256,
        /// Observed stored self-hash.
        observed: Hash256,
    },
    /// FP-reference lineage disagreed with its source checkpoint.
    LineageMismatch {
        /// Mismatched field name.
        field: &'static str,
        /// Expected checkpoint value.
        expected: Hash256,
        /// Observed FP-reference value.
        observed: Hash256,
    },
    /// Self-hash computation expected a top-level object.
    ExpectedObjectForSelfHash,
    /// S4 schema validation failed.
    Schema(S4SchemaError),
    /// JSON serialization failed.
    Json(serde_json::Error),
    /// S4 run-schedule validation failed.
    RunSchedule(S4RunScheduleError),
    /// Canonical JSON serialization failed.
    CanonicalJson(CanonicalJsonError),
    /// Filesystem write failed.
    Io(std::io::Error),
}

impl fmt::Display for S4RunArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema {
                artifact,
                expected,
                observed,
                ..
            } => write!(
                f,
                "{artifact} expected schema {expected:?}, observed {observed:?}"
            ),
            Self::InvalidField { field } => {
                write!(f, "S4 run artifact field {field} violates its RFC pin")
            }
            Self::LossCount { expected, observed } => write!(
                f,
                "s4_gutenberg_run_log.v1 expected {expected} train-loss rows, observed {observed}"
            ),
            Self::EvalPointCount { expected, observed } => write!(
                f,
                "s4_gutenberg_run_log.v1 expected {expected} eval points, observed {observed}"
            ),
            Self::UnexpectedTrainStep {
                index,
                expected,
                observed,
            } => write!(
                f,
                "train-loss row {index} expected step {expected}, observed {observed}"
            ),
            Self::UnexpectedEvalStep {
                index,
                expected,
                observed,
            } => write!(
                f,
                "eval-point row {index} expected step {expected}, observed {observed}"
            ),
            Self::NonFiniteMetric { field, step, .. } => {
                write!(f, "{field} at step {step:?} must be finite")
            }
            Self::NegativeMetric { field, step, value } => write!(
                f,
                "{field} at step {step:?} must be non-negative, observed {value}"
            ),
            Self::InvalidFinalStep { expected, observed } => write!(
                f,
                "s4_gutenberg_checkpoint.v1 expected final_step {expected}, observed {observed}"
            ),
            Self::DivergedRunCannotCheckpoint { observed } => write!(
                f,
                "s4_gutenberg_checkpoint.v1 requires Completed, observed {observed:?}"
            ),
            Self::SelfHashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} mismatch: expected recomputed {expected}, observed {observed}"
            ),
            Self::LineageMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "s4_fp_reference.v1 {field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("S4 run artifact self-hash requires a top-level object")
            }
            Self::Schema(error) => write!(f, "{error}"),
            Self::Json(error) => write!(f, "{error}"),
            Self::RunSchedule(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4RunArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Schema(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::RunSchedule(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}
