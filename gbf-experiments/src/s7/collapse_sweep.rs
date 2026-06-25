//! S7 lambda-switch collapse-sweep helpers.

use std::fmt;

use gbf_artifact::{S7_N_EXPERTS, S7ScoreReport, S7Topology};
use gbf_foundation::{CanonicalJson, CanonicalJsonError, DomainHash, Hash256, SemVer, sha256};
use serde::{Deserialize, Deserializer, Serialize};

use crate::S7_LOG_TARGET;

/// Public event name emitted for each post-Phase-D sweep point.
pub const LAMBDA_SWITCH_SWEEP_STEP_EVENT: &str = "s7.lambda_switch_sweep.step";

/// Public schema id for the S7 router-collapse sweep report.
pub const ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA: &str = "s7_router_collapse_sweep.v1";

/// Schema version carried by `LambdaSwitchSweepRecord`.
pub const LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION: SemVer = SemVer::new(1, 0, 0);

/// D11 lambda-switch sweep grid.
pub const D11_LAMBDA_SWITCH_GRID: [f32; 6] = [0.0, 0.05, 0.1, 0.5, 1.0, 5.0];

/// D11/§9.3 production sweep seed.
pub const D11_LAMBDA_SWITCH_SWEEP_SEED: u64 = 0;

/// D5/D11 production lambda-switch value.
pub const D11_PRODUCTION_LAMBDA_SWITCH: f32 = 0.05;

/// §13.5 RCS-Cadence: each sweep record is produced after 1000 extra steps.
pub const RCS_TRAINING_EXTRA_STEPS: u64 = 1_000;

/// H3 matched-bytes parity bpc margin.
pub const H3_PARITY_MARGIN_BPC: f64 = 0.05;

/// H6-A: production lambda-switch bpc may not regress by more than this margin.
pub const H6_PRODUCTION_BPC_REGRESSION_LIMIT: f64 = 0.05;

/// H6-B: production entropy floor as a fraction of log2(n_experts).
pub const H6_PRODUCTION_ENTROPY_FLOOR_LOG2_RATIO: f32 = 0.85;

/// H6-C/D high-lambda guardrail point.
pub const H6_HIGH_LAMBDA_SWITCH: f32 = 5.0;

/// D11 collapse threshold recorded in `s7_router_collapse_sweep.v1`.
pub const D11_COLLAPSE_THRESHOLD_LAMBDA_SWITCH: f32 = 1.0;

/// H6-C: high lambda must drop entropy by at least this many bits.
pub const H6_HIGH_LAMBDA_ENTROPY_DROP_BITS: f32 = 0.3;

/// H6-D: high lambda must regress eval bpc by at least this much.
pub const H6_HIGH_LAMBDA_BPC_RISE: f64 = 0.3;

const LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION_ID: &str = "1";
const LAMBDA_SWITCH_SWEEP_STEP_SELF_HASH_FIELD: &str = "sweep_self_hash";
const ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA_VERSION_ID: &str = "1";
const ROUTER_COLLAPSE_SWEEP_REPORT_SELF_HASH_FIELD: &str = "sweep_self_hash";
const LAMBDA_SWITCH_GRID_SCHEMA: &str = "s7_lambda_switch_grid.v1";
const LAMBDA_SWITCH_GRID_SCHEMA_VERSION_ID: &str = "1";
const FIXTURE_SWEEP_POINT_SCHEMA: &str = "s7_fixture_lambda_switch_sweep_point.v1";
const FIXTURE_SWEEP_POINT_SCHEMA_VERSION_ID: &str = "1";

/// Completion state for one post-Phase-D lambda-switch sweep point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LambdaSwitchSweepCompletion {
    /// The sweep point completed its 1000 extra steps and produced eval bpc.
    Completed,
    /// The sweep point diverged before the 1000-step target.
    DivergedAt {
        /// First divergent step observed inside the sweep run.
        step: u64,
    },
}

impl LambdaSwitchSweepCompletion {
    const fn divergence_step(self) -> Option<u64> {
        match self {
            Self::Completed => None,
            Self::DivergedAt { step } => Some(step),
        }
    }
}

/// One post-Phase-D lambda-switch sweep record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LambdaSwitchSweepRecord {
    /// Schema version carried by serialized sweep-step records.
    pub schema_version: SemVer,
    /// Experiment seed used by the S7 RouterRng stream.
    pub seed: u64,
    /// Lambda-switch grid point for this sweep record.
    pub lambda_switch: f32,
    /// Base checkpoint training step, normally seed-0 end of Phase D.
    pub base_train_step: u64,
    /// Training step reached after the extra sweep training.
    pub train_step: u64,
    /// Completion state for this sweep point.
    pub completion: LambdaSwitchSweepCompletion,
    /// BPC on the held-out eval subset; `None` if the sweep point diverged.
    pub bpc_eval_subset: Option<f64>,
    /// Mean expert-usage entropy in bits, averaged across layers.
    pub expert_usage_entropy_bits_mean: f32,
    /// Sweep-local bpc delta against the sweep-local production-lambda point.
    pub quality_delta_per_lambda_switch: Option<f64>,
    /// Self-hash over canonical record bytes with this field omitted.
    pub sweep_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for LambdaSwitchSweepRecord {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema_version: SemVer,
            seed: u64,
            lambda_switch: f32,
            base_train_step: u64,
            train_step: u64,
            completion: LambdaSwitchSweepCompletion,
            bpc_eval_subset: Option<f64>,
            expert_usage_entropy_bits_mean: f32,
            quality_delta_per_lambda_switch: Option<f64>,
            sweep_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let record = Self {
            schema_version: raw.schema_version,
            seed: raw.seed,
            lambda_switch: raw.lambda_switch,
            base_train_step: raw.base_train_step,
            train_step: raw.train_step,
            completion: raw.completion,
            bpc_eval_subset: raw.bpc_eval_subset,
            expert_usage_entropy_bits_mean: raw.expert_usage_entropy_bits_mean,
            quality_delta_per_lambda_switch: raw.quality_delta_per_lambda_switch,
            sweep_self_hash: raw.sweep_self_hash,
        };
        record.validate().map_err(serde::de::Error::custom)?;
        Ok(record)
    }
}

impl LambdaSwitchSweepRecord {
    /// Construct a successful post-Phase-D sweep record with the pinned 1000-step delta.
    pub fn successful(
        lambda_switch: f32,
        base_train_step: u64,
        bpc_eval_subset: f64,
        expert_usage_entropy_bits_mean: f32,
        production_bpc_eval_subset: f64,
    ) -> Result<Self, CollapseSweepError> {
        Self::successful_for_seed(
            0,
            lambda_switch,
            base_train_step,
            bpc_eval_subset,
            expert_usage_entropy_bits_mean,
            production_bpc_eval_subset,
        )
    }

    /// Construct a successful post-Phase-D sweep record for a specific S7 seed.
    #[allow(clippy::too_many_arguments)]
    pub fn successful_for_seed(
        seed: u64,
        lambda_switch: f32,
        base_train_step: u64,
        bpc_eval_subset: f64,
        expert_usage_entropy_bits_mean: f32,
        production_bpc_eval_subset: f64,
    ) -> Result<Self, CollapseSweepError> {
        let train_step = base_train_step
            .checked_add(RCS_TRAINING_EXTRA_STEPS)
            .ok_or(CollapseSweepError::TrainStepOverflow {
                base_train_step,
                extra_steps: RCS_TRAINING_EXTRA_STEPS,
            })?;
        Self::from_parts_with_completion_for_seed(
            seed,
            lambda_switch,
            base_train_step,
            train_step,
            LambdaSwitchSweepCompletion::Completed,
            Some(bpc_eval_subset),
            expert_usage_entropy_bits_mean,
            Some(bpc_eval_subset - production_bpc_eval_subset),
        )
    }

    /// Construct a divergent post-Phase-D sweep record with the pinned 1000-step delta.
    pub fn diverged(
        lambda_switch: f32,
        base_train_step: u64,
        divergence_step: u64,
        last_finite_expert_usage_entropy_bits_mean: f32,
    ) -> Result<Self, CollapseSweepError> {
        Self::diverged_for_seed(
            0,
            lambda_switch,
            base_train_step,
            divergence_step,
            last_finite_expert_usage_entropy_bits_mean,
        )
    }

    /// Construct a divergent post-Phase-D sweep record for a specific S7 seed.
    pub fn diverged_for_seed(
        seed: u64,
        lambda_switch: f32,
        base_train_step: u64,
        divergence_step: u64,
        last_finite_expert_usage_entropy_bits_mean: f32,
    ) -> Result<Self, CollapseSweepError> {
        let train_step = base_train_step
            .checked_add(RCS_TRAINING_EXTRA_STEPS)
            .ok_or(CollapseSweepError::TrainStepOverflow {
                base_train_step,
                extra_steps: RCS_TRAINING_EXTRA_STEPS,
            })?;
        Self::from_parts_with_completion_for_seed(
            seed,
            lambda_switch,
            base_train_step,
            train_step,
            LambdaSwitchSweepCompletion::DivergedAt {
                step: divergence_step,
            },
            None,
            last_finite_expert_usage_entropy_bits_mean,
            None,
        )
    }

    /// Construct a completed record from explicit fields and validate D11/D19/§13.5 invariants.
    pub fn from_parts(
        lambda_switch: f32,
        base_train_step: u64,
        train_step: u64,
        bpc_eval_subset: Option<f64>,
        expert_usage_entropy_bits_mean: f32,
        quality_delta_per_lambda_switch: Option<f64>,
    ) -> Result<Self, CollapseSweepError> {
        Self::from_parts_with_completion(
            lambda_switch,
            base_train_step,
            train_step,
            LambdaSwitchSweepCompletion::Completed,
            bpc_eval_subset,
            expert_usage_entropy_bits_mean,
            quality_delta_per_lambda_switch,
        )
    }

    /// Construct a record from explicit fields and validate D11/D19/§13.5 invariants.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts_with_completion(
        lambda_switch: f32,
        base_train_step: u64,
        train_step: u64,
        completion: LambdaSwitchSweepCompletion,
        bpc_eval_subset: Option<f64>,
        expert_usage_entropy_bits_mean: f32,
        quality_delta_per_lambda_switch: Option<f64>,
    ) -> Result<Self, CollapseSweepError> {
        Self::from_parts_with_completion_for_seed(
            0,
            lambda_switch,
            base_train_step,
            train_step,
            completion,
            bpc_eval_subset,
            expert_usage_entropy_bits_mean,
            quality_delta_per_lambda_switch,
        )
    }

    /// Construct a record from explicit fields for a specific S7 seed.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts_with_completion_for_seed(
        seed: u64,
        lambda_switch: f32,
        base_train_step: u64,
        train_step: u64,
        completion: LambdaSwitchSweepCompletion,
        bpc_eval_subset: Option<f64>,
        expert_usage_entropy_bits_mean: f32,
        quality_delta_per_lambda_switch: Option<f64>,
    ) -> Result<Self, CollapseSweepError> {
        let record = Self {
            schema_version: LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION,
            seed,
            lambda_switch: canonicalize_d11_lambda_switch(lambda_switch)?,
            base_train_step,
            train_step,
            completion,
            bpc_eval_subset,
            expert_usage_entropy_bits_mean,
            quality_delta_per_lambda_switch,
            sweep_self_hash: Hash256::ZERO,
        };
        record.with_computed_self_hash()
    }

    /// Return the number of extra training steps beyond the post-Phase-D base checkpoint.
    pub fn training_extra_step_delta(self) -> Result<u64, CollapseSweepError> {
        self.train_step.checked_sub(self.base_train_step).ok_or(
            CollapseSweepError::TrainStepBeforeBase {
                base_train_step: self.base_train_step,
                train_step: self.train_step,
            },
        )
    }

    /// Validate this record against D11/D19/§13.5.
    pub fn validate(self) -> Result<(), CollapseSweepError> {
        self.validate_payload()?;
        self.verify_self_hash()
    }

    /// Compute the canonical self-hash for this sweep step.
    pub fn computed_self_hash(self) -> Result<Hash256, CollapseSweepError> {
        self.validate_payload()?;
        Ok(gbf_foundation::self_hash_omitting_fields(
            Self::domain(),
            &self,
            LAMBDA_SWITCH_SWEEP_STEP_SELF_HASH_FIELD,
            &[],
        )?)
    }

    /// Return a copy with `sweep_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, CollapseSweepError> {
        self.sweep_self_hash = self.computed_self_hash()?;
        self.validate()?;
        Ok(self)
    }

    /// Verify that the stored self-hash matches the record payload.
    pub fn verify_self_hash(self) -> Result<(), CollapseSweepError> {
        let expected = self.computed_self_hash()?;
        if self.sweep_self_hash != expected {
            return Err(CollapseSweepError::SelfHashMismatch {
                field: LAMBDA_SWITCH_SWEEP_STEP_SELF_HASH_FIELD,
                expected,
                observed: self.sweep_self_hash,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes for this sweep step.
    pub fn canonical_json_bytes(self) -> Result<Vec<u8>, CollapseSweepError> {
        self.validate()?;
        Ok(CanonicalJson::to_vec(&self)?)
    }

    /// Canonical JSON string for trace subscribers.
    pub fn canonical_json_string(self) -> Result<String, CollapseSweepError> {
        String::from_utf8(self.canonical_json_bytes()?).map_err(|error| {
            CollapseSweepError::CanonicalJsonUtf8 {
                detail: error.to_string(),
            }
        })
    }

    /// Emit this LambdaSwitchSweepStep through tracing.
    pub fn emit_trace(self) -> Result<(), CollapseSweepError> {
        self.validate()?;
        let sweep_canonical_json = self.canonical_json_string()?;
        let completion = match self.completion {
            LambdaSwitchSweepCompletion::Completed => "completed",
            LambdaSwitchSweepCompletion::DivergedAt { .. } => "diverged_at",
        };
        tracing::info!(
            target: S7_LOG_TARGET,
            event_name = LAMBDA_SWITCH_SWEEP_STEP_EVENT,
            schema_version_major = self.schema_version.major,
            schema_version_minor = self.schema_version.minor,
            schema_version_patch = self.schema_version.patch,
            seed = self.seed,
            lambda_switch = self.lambda_switch,
            base_train_step = self.base_train_step,
            extra_train_steps = RCS_TRAINING_EXTRA_STEPS,
            train_step = self.train_step,
            completion = completion,
            divergence_step = self.completion.divergence_step(),
            bpc_eval_subset = self.bpc_eval_subset,
            expert_usage_entropy_bits_mean = self.expert_usage_entropy_bits_mean,
            quality_delta_per_lambda_switch = self.quality_delta_per_lambda_switch,
            sweep_self_hash = %self.sweep_self_hash,
            sweep_canonical_json = sweep_canonical_json.as_str(),
            "s7 lambda-switch sweep step"
        );
        Ok(())
    }

    /// Domain used for canonical self-hashing.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "LambdaSwitchSweepRecord",
            "s7_lambda_switch_sweep_step.v1",
            LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION_ID,
        )
    }

    fn validate_payload(self) -> Result<(), CollapseSweepError> {
        if self.schema_version != LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION {
            return Err(CollapseSweepError::UnexpectedSchemaVersion {
                expected: LAMBDA_SWITCH_SWEEP_STEP_SCHEMA_VERSION,
                observed: self.schema_version,
            });
        }
        let _grid_index = lambda_switch_grid_index(self.lambda_switch)?;
        let observed_delta = self.training_extra_step_delta()?;
        if observed_delta != RCS_TRAINING_EXTRA_STEPS {
            return Err(CollapseSweepError::UnexpectedTrainingExtraStepDelta {
                lambda_switch: self.lambda_switch,
                observed: observed_delta,
                expected: RCS_TRAINING_EXTRA_STEPS,
            });
        }
        match self.completion {
            LambdaSwitchSweepCompletion::Completed => {
                if self.bpc_eval_subset.is_none() {
                    return Err(CollapseSweepError::CompletedRecordMissingBpc {
                        lambda_switch: self.lambda_switch,
                    });
                }
                if self.quality_delta_per_lambda_switch.is_none() {
                    return Err(CollapseSweepError::CompletedRecordMissingQualityDelta {
                        lambda_switch: self.lambda_switch,
                    });
                }
            }
            LambdaSwitchSweepCompletion::DivergedAt { step } => {
                if step <= self.base_train_step || step > self.train_step {
                    return Err(CollapseSweepError::DivergenceStepOutOfRange {
                        lambda_switch: self.lambda_switch,
                        base_train_step: self.base_train_step,
                        train_step: self.train_step,
                        divergence_step: step,
                    });
                }
                if self.bpc_eval_subset.is_some() {
                    return Err(CollapseSweepError::DivergedRecordHasBpc {
                        lambda_switch: self.lambda_switch,
                    });
                }
                if self.quality_delta_per_lambda_switch.is_some() {
                    return Err(CollapseSweepError::DivergedRecordHasQualityDelta {
                        lambda_switch: self.lambda_switch,
                    });
                }
            }
        }
        if let Some(bpc) = self.bpc_eval_subset {
            validate_finite_nonnegative_f64("bpc_eval_subset", bpc)?;
        }
        validate_finite_nonnegative_f32(
            "expert_usage_entropy_bits_mean",
            self.expert_usage_entropy_bits_mean,
        )?;
        validate_entropy_bits_mean(self.expert_usage_entropy_bits_mean)?;
        if let Some(delta) = self.quality_delta_per_lambda_switch {
            validate_finite_f64("quality_delta_per_lambda_switch", delta)?;
        }
        Ok(())
    }
}

/// Validate that the sweep has exactly one 1000-extra-step record per D11 grid point.
pub fn validate_collapse_sweep_records(
    records: &[LambdaSwitchSweepRecord],
) -> Result<(), CollapseSweepError> {
    if records.len() != D11_LAMBDA_SWITCH_GRID.len() {
        return Err(CollapseSweepError::UnexpectedRecordCount {
            observed: records.len(),
            expected: D11_LAMBDA_SWITCH_GRID.len(),
        });
    }

    let expected_seed = records
        .first()
        .expect("length checked against non-empty D11 grid")
        .seed;
    let expected_base_train_step = records
        .first()
        .expect("length checked against non-empty D11 grid")
        .base_train_step;
    let mut seen = [false; D11_LAMBDA_SWITCH_GRID.len()];
    for record in records {
        record.validate()?;
        if record.seed != expected_seed {
            return Err(CollapseSweepError::RecordSeedMismatch {
                lambda_switch: record.lambda_switch,
                expected: expected_seed,
                observed: record.seed,
            });
        }
        if record.base_train_step != expected_base_train_step {
            return Err(CollapseSweepError::BaseTrainStepMismatch {
                lambda_switch: record.lambda_switch,
                expected: expected_base_train_step,
                observed: record.base_train_step,
            });
        }
        let index = lambda_switch_grid_index(record.lambda_switch)?;
        if seen[index] {
            return Err(CollapseSweepError::DuplicateGridRecord {
                lambda_switch: record.lambda_switch,
            });
        }
        seen[index] = true;
    }

    for (index, seen) in seen.iter().copied().enumerate() {
        if !seen {
            return Err(CollapseSweepError::MissingGridRecord {
                lambda_switch: D11_LAMBDA_SWITCH_GRID[index],
            });
        }
    }

    Ok(())
}

/// H6 router-collapse guardrail verdict.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum GuardrailVerdict {
    /// A/B/C/D all hold.
    Pass,
    /// A failed: production lambda-switch regressed bpc by more than 0.05.
    FailA,
    /// B failed: production entropy fell below 0.85 * log2(n_experts).
    FailB,
    /// C failed: high-lambda entropy did not drop by at least 0.3 bits.
    FailC,
    /// D failed: high-lambda bpc did not rise by at least 0.3.
    FailD,
    /// A sweep point diverged and no high-lambda collapse-credit recovery applied.
    InconclusiveDiverged {
        /// Divergent lambda-switch grid point.
        lambda_switch: f32,
        /// First divergent sweep step.
        step: u64,
    },
}

/// Compute the H6 production entropy floor in bits.
pub fn h6_production_entropy_floor_bits() -> f32 {
    H6_PRODUCTION_ENTROPY_FLOOR_LOG2_RATIO * f32::from(S7_N_EXPERTS).log2()
}

/// Derive the H6 guardrail verdict from a validated D11 lambda-switch sweep.
pub fn h6_guardrail_verdict(
    records: &[LambdaSwitchSweepRecord],
) -> Result<GuardrailVerdict, CollapseSweepError> {
    validate_collapse_sweep_records(records)?;

    let baseline = required_record(records, 0.0)?;
    let production = required_record(records, D11_PRODUCTION_LAMBDA_SWITCH)?;
    let high = required_record(records, H6_HIGH_LAMBDA_SWITCH)?;

    for record in records {
        if record.lambda_switch.to_bits() == H6_HIGH_LAMBDA_SWITCH.to_bits() {
            continue;
        }
        if let Some(step) = record.completion.divergence_step() {
            return Ok(GuardrailVerdict::InconclusiveDiverged {
                lambda_switch: record.lambda_switch,
                step,
            });
        }
    }

    let bpc_baseline = baseline
        .bpc_eval_subset
        .expect("validated completed baseline record has bpc");
    let bpc_production = production
        .bpc_eval_subset
        .expect("validated completed production record has bpc");
    let ent_production = production.expert_usage_entropy_bits_mean;

    if bpc_production - bpc_baseline > H6_PRODUCTION_BPC_REGRESSION_LIMIT {
        return Ok(GuardrailVerdict::FailA);
    }
    if ent_production < h6_production_entropy_floor_bits() {
        return Ok(GuardrailVerdict::FailB);
    }

    let ent_high = high.expert_usage_entropy_bits_mean;
    let high_entropy_collapse = ent_production - ent_high >= H6_HIGH_LAMBDA_ENTROPY_DROP_BITS;
    if let Some(step) = high.completion.divergence_step() {
        if high_entropy_collapse {
            // RFC A23: high-lambda divergence supplies the quality-regression
            // evidence; recovery only needs last-finite entropy collapse.
            return Ok(GuardrailVerdict::Pass);
        }
        return Ok(GuardrailVerdict::InconclusiveDiverged {
            lambda_switch: high.lambda_switch,
            step,
        });
    }

    if !high_entropy_collapse {
        return Ok(GuardrailVerdict::FailC);
    }

    let bpc_high = high
        .bpc_eval_subset
        .expect("validated completed high-lambda record has bpc");
    if bpc_high - bpc_production < H6_HIGH_LAMBDA_BPC_RISE {
        return Ok(GuardrailVerdict::FailD);
    }

    Ok(GuardrailVerdict::Pass)
}

/// Validated `s7_router_collapse_sweep.v1` artifact payload.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RouterCollapseSweepReport {
    /// Schema literal.
    pub schema: String,
    /// Experiment seed used by the S7 RouterRng stream.
    pub seed: u64,
    /// End-of-Phase-D base checkpoint hash.
    pub base_checkpoint_sha: Hash256,
    /// Pinned D11 lambda-switch grid in exact order.
    pub grid: Vec<f32>,
    /// One validated LambdaSwitchSweepStep per grid point.
    pub records: Vec<LambdaSwitchSweepRecord>,
    /// Production lambda-switch value.
    pub production_lambda: f32,
    /// Collapse-threshold lambda-switch value.
    pub collapse_threshold: f32,
    /// H6 guardrail verdict derived from `records`.
    pub guardrail_verdict: GuardrailVerdict,
    /// Self-hash over canonical report bytes with this field omitted.
    pub sweep_self_hash: Hash256,
}

impl<'de> Deserialize<'de> for RouterCollapseSweepReport {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Raw {
            schema: String,
            seed: u64,
            base_checkpoint_sha: Hash256,
            grid: Vec<f32>,
            records: Vec<LambdaSwitchSweepRecord>,
            production_lambda: f32,
            collapse_threshold: f32,
            guardrail_verdict: GuardrailVerdict,
            sweep_self_hash: Hash256,
        }

        let raw = Raw::deserialize(deserializer)?;
        let report = Self {
            schema: raw.schema,
            seed: raw.seed,
            base_checkpoint_sha: raw.base_checkpoint_sha,
            grid: raw.grid,
            records: raw.records,
            production_lambda: raw.production_lambda,
            collapse_threshold: raw.collapse_threshold,
            guardrail_verdict: raw.guardrail_verdict,
            sweep_self_hash: raw.sweep_self_hash,
        };
        report.validate().map_err(serde::de::Error::custom)?;
        Ok(report)
    }
}

impl RouterCollapseSweepReport {
    /// Construct a report for the canonical D11 grid.
    pub fn new(
        seed: u64,
        base_checkpoint_sha: Hash256,
        records: Vec<LambdaSwitchSweepRecord>,
    ) -> Result<Self, CollapseSweepError> {
        Self::from_grid_records(
            seed,
            base_checkpoint_sha,
            D11_LAMBDA_SWITCH_GRID.to_vec(),
            records,
        )
    }

    /// Construct a report from explicit grid and records.
    pub fn from_grid_records(
        seed: u64,
        base_checkpoint_sha: Hash256,
        grid: Vec<f32>,
        records: Vec<LambdaSwitchSweepRecord>,
    ) -> Result<Self, CollapseSweepError> {
        let grid = canonicalize_d11_lambda_switch_grid(&grid)?;
        validate_collapse_sweep_records(&records)?;
        let guardrail_verdict = h6_guardrail_verdict(&records)?;
        let report = Self {
            schema: ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA.to_owned(),
            seed,
            base_checkpoint_sha,
            grid,
            records,
            production_lambda: D11_PRODUCTION_LAMBDA_SWITCH,
            collapse_threshold: D11_COLLAPSE_THRESHOLD_LAMBDA_SWITCH,
            guardrail_verdict,
            sweep_self_hash: Hash256::ZERO,
        };
        report.with_computed_self_hash()
    }

    /// Validate report-level RCS invariants and self-hash.
    pub fn validate(&self) -> Result<(), CollapseSweepError> {
        self.validate_payload()?;
        self.verify_self_hash()
    }

    /// Compute the canonical self-hash for this report.
    pub fn computed_self_hash(&self) -> Result<Hash256, CollapseSweepError> {
        self.validate_payload()?;
        Ok(gbf_foundation::self_hash_omitting_fields(
            Self::domain(),
            self,
            ROUTER_COLLAPSE_SWEEP_REPORT_SELF_HASH_FIELD,
            &[],
        )?)
    }

    /// Return a copy with `sweep_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, CollapseSweepError> {
        self.sweep_self_hash = self.computed_self_hash()?;
        self.validate()?;
        Ok(self)
    }

    /// Verify that the stored self-hash matches the report payload.
    pub fn verify_self_hash(&self) -> Result<(), CollapseSweepError> {
        let expected = self.computed_self_hash()?;
        if self.sweep_self_hash != expected {
            return Err(CollapseSweepError::SelfHashMismatch {
                field: ROUTER_COLLAPSE_SWEEP_REPORT_SELF_HASH_FIELD,
                expected,
                observed: self.sweep_self_hash,
            });
        }
        Ok(())
    }

    /// Canonical JSON bytes for this report.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, CollapseSweepError> {
        self.validate()?;
        Ok(CanonicalJson::to_vec(self)?)
    }

    /// Domain used for canonical self-hashing.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "RouterCollapseSweepReport",
            ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA,
            ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA_VERSION_ID,
        )
    }

    fn validate_payload(&self) -> Result<(), CollapseSweepError> {
        if self.schema != ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA {
            return Err(CollapseSweepError::UnexpectedSchema {
                expected: ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA,
                observed: self.schema.clone(),
            });
        }
        canonicalize_d11_lambda_switch_grid(&self.grid)?;
        validate_collapse_sweep_records(&self.records)?;
        if self.production_lambda.to_bits() != D11_PRODUCTION_LAMBDA_SWITCH.to_bits() {
            return Err(CollapseSweepError::UnexpectedProductionLambda {
                observed: self.production_lambda,
                expected: D11_PRODUCTION_LAMBDA_SWITCH,
            });
        }
        if self.collapse_threshold.to_bits() != D11_COLLAPSE_THRESHOLD_LAMBDA_SWITCH.to_bits() {
            return Err(CollapseSweepError::UnexpectedCollapseThreshold {
                observed: self.collapse_threshold,
                expected: D11_COLLAPSE_THRESHOLD_LAMBDA_SWITCH,
            });
        }
        for record in &self.records {
            if record.seed != self.seed {
                return Err(CollapseSweepError::RecordSeedMismatch {
                    lambda_switch: record.lambda_switch,
                    expected: self.seed,
                    observed: record.seed,
                });
            }
        }
        let expected_verdict = h6_guardrail_verdict(&self.records)?;
        if self.guardrail_verdict != expected_verdict {
            return Err(CollapseSweepError::GuardrailVerdictMismatch {
                expected: expected_verdict,
                observed: self.guardrail_verdict,
            });
        }
        Ok(())
    }
}

/// Inputs shared by every point in one post-Phase-D sweep.
#[derive(Debug, Clone, PartialEq)]
pub struct LambdaSwitchSweepInput {
    /// Experiment seed used by the S7 RouterRng stream.
    pub seed: u64,
    /// End-of-Phase-D base checkpoint hash.
    pub base_checkpoint_sha: Hash256,
    /// End-of-Phase-D base checkpoint train step.
    pub base_train_step: u64,
    /// SHA-256 of the exact validation byte subset scored by each sweep point.
    pub val_eval_subset_sha: Hash256,
    /// Byte length of the validation subset.
    pub val_eval_subset_len: u64,
    /// Extra retraining steps each sweep point must run from the same base checkpoint.
    pub extra_train_steps: u64,
    /// Lambda-switch grid to execute.
    pub grid: Vec<f32>,
}

impl LambdaSwitchSweepInput {
    /// Construct inputs for the canonical D11 grid.
    pub fn d11(
        seed: u64,
        base_checkpoint_sha: Hash256,
        base_train_step: u64,
        val_eval_subset_sha: Hash256,
        val_eval_subset_len: u64,
    ) -> Result<Self, CollapseSweepError> {
        let input = Self {
            seed,
            base_checkpoint_sha,
            base_train_step,
            val_eval_subset_sha,
            val_eval_subset_len,
            extra_train_steps: RCS_TRAINING_EXTRA_STEPS,
            grid: D11_LAMBDA_SWITCH_GRID.to_vec(),
        };
        input.validate()?;
        Ok(input)
    }

    /// Construct D11 inputs and hash the exact validation subset bytes.
    pub fn d11_from_val_eval_subset_bytes(
        seed: u64,
        base_checkpoint_sha: Hash256,
        base_train_step: u64,
        val_eval_subset: &[u8],
    ) -> Result<Self, CollapseSweepError> {
        Self::d11(
            seed,
            base_checkpoint_sha,
            base_train_step,
            sha256(val_eval_subset),
            val_eval_subset.len() as u64,
        )
    }

    /// Validate the production-facing sweep descriptor before any producer runs.
    pub fn validate(&self) -> Result<(), CollapseSweepError> {
        if self.seed != D11_LAMBDA_SWITCH_SWEEP_SEED {
            return Err(CollapseSweepError::UnexpectedSweepSeed {
                observed: self.seed,
                expected: D11_LAMBDA_SWITCH_SWEEP_SEED,
            });
        }
        if self.base_checkpoint_sha == Hash256::ZERO {
            return Err(CollapseSweepError::MissingBaseCheckpointHash);
        }
        if self.val_eval_subset_sha == Hash256::ZERO {
            return Err(CollapseSweepError::MissingValEvalSubsetHash);
        }
        if self.val_eval_subset_len == 0 {
            return Err(CollapseSweepError::EmptyValEvalSubset);
        }
        if self.extra_train_steps != RCS_TRAINING_EXTRA_STEPS {
            return Err(CollapseSweepError::UnexpectedSweepExtraTrainSteps {
                observed: self.extra_train_steps,
                expected: RCS_TRAINING_EXTRA_STEPS,
            });
        }
        canonicalize_d11_lambda_switch_grid(&self.grid)?;
        Ok(())
    }
}

/// Input for one producer-owned retrain-and-score sweep point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LambdaSwitchSweepPointInput {
    /// Experiment seed used by the S7 RouterRng stream.
    pub seed: u64,
    /// End-of-Phase-D base checkpoint hash.
    pub base_checkpoint_sha: Hash256,
    /// End-of-Phase-D base checkpoint train step.
    pub base_train_step: u64,
    /// SHA-256 of the exact validation byte subset scored by this sweep point.
    pub val_eval_subset_sha: Hash256,
    /// Byte length of the validation subset.
    pub val_eval_subset_len: u64,
    /// Required extra retraining steps for this point.
    pub extra_train_steps: u64,
    /// Canonical D11 lambda-switch value for this point.
    pub lambda_switch: f32,
    /// Hash of the exact D11 grid bit patterns.
    pub lambda_switch_grid_hash: Hash256,
}

impl LambdaSwitchSweepPointInput {
    /// Validate the per-point producer contract passed to retrain/score implementations.
    pub fn validate(self) -> Result<(), CollapseSweepError> {
        if self.seed != D11_LAMBDA_SWITCH_SWEEP_SEED {
            return Err(CollapseSweepError::UnexpectedSweepSeed {
                observed: self.seed,
                expected: D11_LAMBDA_SWITCH_SWEEP_SEED,
            });
        }
        if self.base_checkpoint_sha == Hash256::ZERO {
            return Err(CollapseSweepError::MissingBaseCheckpointHash);
        }
        if self.val_eval_subset_sha == Hash256::ZERO {
            return Err(CollapseSweepError::MissingValEvalSubsetHash);
        }
        if self.val_eval_subset_len == 0 {
            return Err(CollapseSweepError::EmptyValEvalSubset);
        }
        if self.extra_train_steps != RCS_TRAINING_EXTRA_STEPS {
            return Err(CollapseSweepError::UnexpectedSweepExtraTrainSteps {
                observed: self.extra_train_steps,
                expected: RCS_TRAINING_EXTRA_STEPS,
            });
        }
        canonicalize_d11_lambda_switch(self.lambda_switch)?;
        let expected_grid_hash = lambda_switch_grid_hash(&D11_LAMBDA_SWITCH_GRID)?;
        if self.lambda_switch_grid_hash != expected_grid_hash {
            return Err(CollapseSweepError::UnexpectedLambdaSwitchGridHash {
                observed: self.lambda_switch_grid_hash,
                expected: expected_grid_hash,
            });
        }
        Ok(())
    }
}

/// Producer result for one 1000-extra-step sweep point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LambdaSwitchSweepPointOutcome {
    /// Completion state observed by the producer.
    pub completion: LambdaSwitchSweepCompletion,
    /// BPC on the held-out eval subset; `None` iff divergent.
    pub bpc_eval_subset: Option<f64>,
    /// Mean expert-usage entropy in bits, averaged across layers.
    pub expert_usage_entropy_bits_mean: f32,
}

impl LambdaSwitchSweepPointOutcome {
    /// Construct a completed point outcome.
    pub fn completed(
        bpc_eval_subset: f64,
        expert_usage_entropy_bits_mean: f32,
    ) -> Result<Self, CollapseSweepError> {
        validate_finite_nonnegative_f64("bpc_eval_subset", bpc_eval_subset)?;
        validate_finite_nonnegative_f32(
            "expert_usage_entropy_bits_mean",
            expert_usage_entropy_bits_mean,
        )?;
        validate_entropy_bits_mean(expert_usage_entropy_bits_mean)?;
        Ok(Self {
            completion: LambdaSwitchSweepCompletion::Completed,
            bpc_eval_subset: Some(bpc_eval_subset),
            expert_usage_entropy_bits_mean,
        })
    }

    /// Construct a divergent point outcome using the last finite entropy value.
    pub fn diverged_at(
        step: u64,
        last_finite_expert_usage_entropy_bits_mean: f32,
    ) -> Result<Self, CollapseSweepError> {
        validate_finite_nonnegative_f32(
            "expert_usage_entropy_bits_mean",
            last_finite_expert_usage_entropy_bits_mean,
        )?;
        validate_entropy_bits_mean(last_finite_expert_usage_entropy_bits_mean)?;
        Ok(Self {
            completion: LambdaSwitchSweepCompletion::DivergedAt { step },
            bpc_eval_subset: None,
            expert_usage_entropy_bits_mean: last_finite_expert_usage_entropy_bits_mean,
        })
    }
}

/// Producer boundary for the real checkpoint retrain/score implementation.
pub trait LambdaSwitchSweepProducer {
    /// Retrain from `base_checkpoint_sha` for the point's 1000 extra steps and score it.
    ///
    /// Implementations must hold non-`lambda_switch` loss weights at their
    /// production values, score the validation subset identified by
    /// `val_eval_subset_sha`, and return entropy averaged across all router
    /// layers (or the last finite layer-averaged entropy on divergence).
    fn run_sweep_point(
        &self,
        input: LambdaSwitchSweepPointInput,
    ) -> Result<LambdaSwitchSweepPointOutcome, CollapseSweepError>;
}

/// Deterministic tiny-fixture producer used by smoke tests until real training is wired.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeterministicFixtureSweepProducer;

impl LambdaSwitchSweepProducer for DeterministicFixtureSweepProducer {
    fn run_sweep_point(
        &self,
        input: LambdaSwitchSweepPointInput,
    ) -> Result<LambdaSwitchSweepPointOutcome, CollapseSweepError> {
        input.validate()?;
        let grid_index = lambda_switch_grid_index(input.lambda_switch)?;
        let bpc_offset = deterministic_fixture_offset(input, "bpc", 0.001)?;
        let entropy_offset = deterministic_fixture_offset(input, "entropy", 0.01)? as f32;
        let (nominal_bpc, nominal_entropy) = match grid_index {
            0 => (1.000, 1.92_f32),
            1 => (1.020, 1.86_f32),
            2 => (1.030, 1.79_f32),
            3 => (1.070, 1.66_f32),
            4 => (1.160, 1.50_f32),
            5 => (1.410, 1.32_f32),
            _ => unreachable!("grid index is bounded by D11_LAMBDA_SWITCH_GRID"),
        };

        LambdaSwitchSweepPointOutcome::completed(
            nominal_bpc + bpc_offset,
            nominal_entropy + entropy_offset,
        )
    }
}

/// Run the D11 collapse sweep with a producer and emit one trace event per point.
pub fn run_lambda_switch_sweep<P>(
    input: &LambdaSwitchSweepInput,
    producer: &P,
) -> Result<RouterCollapseSweepReport, CollapseSweepError>
where
    P: LambdaSwitchSweepProducer,
{
    input.validate()?;
    let grid = canonicalize_d11_lambda_switch_grid(&input.grid)?;
    let lambda_switch_grid_hash = lambda_switch_grid_hash(&grid)?;
    let mut outcomes = Vec::with_capacity(grid.len());

    for lambda_switch in grid.iter().copied() {
        let point_input = LambdaSwitchSweepPointInput {
            seed: input.seed,
            base_checkpoint_sha: input.base_checkpoint_sha,
            base_train_step: input.base_train_step,
            val_eval_subset_sha: input.val_eval_subset_sha,
            val_eval_subset_len: input.val_eval_subset_len,
            extra_train_steps: input.extra_train_steps,
            lambda_switch,
            lambda_switch_grid_hash,
        };
        point_input.validate()?;
        let outcome = producer.run_sweep_point(point_input)?;
        outcomes.push((lambda_switch, outcome));
    }

    let production_bpc = outcomes
        .iter()
        .find(|(lambda_switch, _)| {
            lambda_switch.to_bits() == D11_PRODUCTION_LAMBDA_SWITCH.to_bits()
        })
        .and_then(|(_, outcome)| outcome.bpc_eval_subset)
        .ok_or(CollapseSweepError::ProductionSweepPointDidNotComplete)?;

    let mut records = Vec::with_capacity(outcomes.len());
    for (lambda_switch, outcome) in outcomes {
        let record = match outcome.completion {
            LambdaSwitchSweepCompletion::Completed => LambdaSwitchSweepRecord::successful_for_seed(
                input.seed,
                lambda_switch,
                input.base_train_step,
                outcome
                    .bpc_eval_subset
                    .expect("completed outcome constructor stores bpc"),
                outcome.expert_usage_entropy_bits_mean,
                production_bpc,
            )?,
            LambdaSwitchSweepCompletion::DivergedAt { step } => {
                LambdaSwitchSweepRecord::diverged_for_seed(
                    input.seed,
                    lambda_switch,
                    input.base_train_step,
                    step,
                    outcome.expert_usage_entropy_bits_mean,
                )?
            }
        };
        record.emit_trace()?;
        records.push(record);
    }

    RouterCollapseSweepReport::from_grid_records(
        input.seed,
        input.base_checkpoint_sha,
        grid,
        records,
    )
}

/// Return the H6 verdict for the F8-broken constant-lambda falsifier.
///
/// This helper intentionally does not validate the D11 RCS artifact shape: the
/// F8 substitute is the invalid downstream case where the grid contains only
/// the production lambda, so the high-lambda entropy drop is defined as zero.
pub fn f8_constant_lambda_sweep_verdict(
    records: &[LambdaSwitchSweepRecord],
) -> Result<GuardrailVerdict, CollapseSweepError> {
    if records.is_empty() {
        return Err(CollapseSweepError::DegenerateSweepEmpty);
    }
    for record in records {
        record.validate()?;
        if record.lambda_switch.to_bits() != D11_PRODUCTION_LAMBDA_SWITCH.to_bits() {
            return Err(CollapseSweepError::DegenerateSweepNonProductionLambda {
                lambda_switch: record.lambda_switch,
            });
        }
    }
    Ok(GuardrailVerdict::FailC)
}

/// H3 per-seed parity verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum S7ParitySeedVerdict {
    /// MoE beats dense by the required strict margin.
    Pass,
    /// MoE failed the strict per-seed margin.
    Fail,
}

/// Compute §11.1 H3 parity from production-run `S7ScoreReport`s only.
pub fn s7_parity_seed_from_production_scores(
    production_moe_score: &S7ScoreReport,
    production_dense_matched_score: &S7ScoreReport,
) -> Result<S7ParitySeedVerdict, CollapseSweepError> {
    s7_parity_seed_from_production_scores_with_margin(
        production_moe_score,
        production_dense_matched_score,
        H3_PARITY_MARGIN_BPC,
    )
}

/// Compute §11.1 H3 parity from production-run `S7ScoreReport`s with an explicit margin.
pub fn s7_parity_seed_from_production_scores_with_margin(
    production_moe_score: &S7ScoreReport,
    production_dense_matched_score: &S7ScoreReport,
    margin_bpc: f64,
) -> Result<S7ParitySeedVerdict, CollapseSweepError> {
    validate_finite_nonnegative_f64("margin_bpc", margin_bpc)?;
    validate_score_pair(production_moe_score, production_dense_matched_score)?;

    if production_moe_score.bpc < production_dense_matched_score.bpc - margin_bpc {
        Ok(S7ParitySeedVerdict::Pass)
    } else {
        Ok(S7ParitySeedVerdict::Fail)
    }
}

/// Errors returned by S7 collapse-sweep helpers.
#[derive(Debug, Clone, PartialEq)]
pub enum CollapseSweepError {
    /// The sweep descriptor used a seed other than the RFC-pinned seed 0.
    UnexpectedSweepSeed {
        /// Observed seed.
        observed: u64,
        /// Expected seed.
        expected: u64,
    },
    /// The sweep descriptor did not identify a real end-of-Phase-D checkpoint.
    MissingBaseCheckpointHash,
    /// The sweep descriptor did not identify the validation subset.
    MissingValEvalSubsetHash,
    /// The sweep descriptor carried an empty validation subset.
    EmptyValEvalSubset,
    /// The sweep descriptor requested an unsupported number of extra retraining steps.
    UnexpectedSweepExtraTrainSteps {
        /// Observed extra retraining steps.
        observed: u64,
        /// Expected extra retraining steps.
        expected: u64,
    },
    /// The per-point producer input did not carry the exact D11 grid hash.
    UnexpectedLambdaSwitchGridHash {
        /// Observed grid hash.
        observed: Hash256,
        /// Expected grid hash.
        expected: Hash256,
    },
    /// The lambda-switch value is not in the pinned D11 grid.
    LambdaSwitchNotInGrid {
        /// Observed lambda-switch value.
        lambda_switch: f32,
    },
    /// The report grid length did not match the pinned D11 grid length.
    UnexpectedGridCount {
        /// Observed number of grid entries.
        observed: usize,
        /// Expected number of grid entries.
        expected: usize,
    },
    /// A report grid value did not match the pinned D11 bit pattern at that index.
    UnexpectedGridValue {
        /// Grid index.
        index: usize,
        /// Observed lambda-switch value.
        observed: f32,
        /// Expected lambda-switch value.
        expected: f32,
    },
    /// The record count did not match the D11 grid length.
    UnexpectedRecordCount {
        /// Observed number of records.
        observed: usize,
        /// Expected number of records.
        expected: usize,
    },
    /// More than one record exists for a D11 grid point.
    DuplicateGridRecord {
        /// Duplicated lambda-switch value.
        lambda_switch: f32,
    },
    /// A D11 grid point is missing a record.
    MissingGridRecord {
        /// Missing lambda-switch value.
        lambda_switch: f32,
    },
    /// Records in a single sweep used different S7 seeds.
    RecordSeedMismatch {
        /// Lambda-switch value for the mismatched record.
        lambda_switch: f32,
        /// Expected S7 seed.
        expected: u64,
        /// Observed S7 seed.
        observed: u64,
    },
    /// Records in a single sweep used different base train steps.
    BaseTrainStepMismatch {
        /// Lambda-switch value for the mismatched record.
        lambda_switch: f32,
        /// Expected base train step.
        expected: u64,
        /// Observed base train step.
        observed: u64,
    },
    /// The record was produced at an unexpected extra-step delta.
    UnexpectedTrainingExtraStepDelta {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
        /// Observed extra training steps.
        observed: u64,
        /// Expected extra training steps.
        expected: u64,
    },
    /// The explicit train step came before the base checkpoint step.
    TrainStepBeforeBase {
        /// Base checkpoint step.
        base_train_step: u64,
        /// Observed train step.
        train_step: u64,
    },
    /// Adding extra steps to the base train step overflowed.
    TrainStepOverflow {
        /// Base checkpoint step.
        base_train_step: u64,
        /// Extra sweep steps.
        extra_steps: u64,
    },
    /// A completed sweep record lacked `bpc_eval_subset`.
    CompletedRecordMissingBpc {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
    },
    /// A completed sweep record lacked `quality_delta_per_lambda_switch`.
    CompletedRecordMissingQualityDelta {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
    },
    /// A divergent sweep record included `bpc_eval_subset`, which must be null.
    DivergedRecordHasBpc {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
    },
    /// A divergent sweep record included `quality_delta_per_lambda_switch`, which must be null.
    DivergedRecordHasQualityDelta {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
    },
    /// A divergent sweep step was outside the 1000-step sweep window.
    DivergenceStepOutOfRange {
        /// Lambda-switch value for the bad record.
        lambda_switch: f32,
        /// Base checkpoint step.
        base_train_step: u64,
        /// Sweep target step.
        train_step: u64,
        /// Divergence step.
        divergence_step: u64,
    },
    /// A floating-point field was not finite.
    NonFiniteFloat {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// A nonnegative floating-point field was negative.
    NegativeFloat {
        /// Field name.
        field: &'static str,
        /// Observed value.
        value: f64,
    },
    /// Entropy exceeded the legal expert-usage range.
    EntropyBitsOutOfRange {
        /// Observed entropy value.
        value: f32,
        /// Maximum legal entropy in bits.
        max: f32,
    },
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson {
        /// Error detail.
        detail: String,
    },
    /// Canonical JSON bytes were not UTF-8.
    CanonicalJsonUtf8 {
        /// Error detail.
        detail: String,
    },
    /// The stored schema literal did not match the pinned report schema.
    UnexpectedSchema {
        /// Expected schema literal.
        expected: &'static str,
        /// Observed schema literal.
        observed: String,
    },
    /// The stored schema version did not match the pinned sweep-step schema.
    UnexpectedSchemaVersion {
        /// Expected schema version.
        expected: SemVer,
        /// Observed schema version.
        observed: SemVer,
    },
    /// The stored self-hash did not match the payload.
    SelfHashMismatch {
        /// Self-hash field name.
        field: &'static str,
        /// Expected self-hash.
        expected: Hash256,
        /// Observed self-hash.
        observed: Hash256,
    },
    /// The report production lambda did not match D11/D5.
    UnexpectedProductionLambda {
        /// Observed production lambda value.
        observed: f32,
        /// Expected production lambda value.
        expected: f32,
    },
    /// The report collapse threshold did not match D11.
    UnexpectedCollapseThreshold {
        /// Observed collapse threshold.
        observed: f32,
        /// Expected collapse threshold.
        expected: f32,
    },
    /// The report guardrail verdict did not match the records.
    GuardrailVerdictMismatch {
        /// Expected guardrail verdict.
        expected: GuardrailVerdict,
        /// Observed guardrail verdict.
        observed: GuardrailVerdict,
    },
    /// The production sweep point did not complete, so quality deltas cannot be computed.
    ProductionSweepPointDidNotComplete,
    /// The F8 degenerate sweep had no production-lambda records.
    DegenerateSweepEmpty,
    /// The F8 degenerate sweep contained a non-production lambda.
    DegenerateSweepNonProductionLambda {
        /// Observed lambda-switch value.
        lambda_switch: f32,
    },
    /// The two production score reports did not use the same seed.
    ScoreSeedMismatch {
        /// MoE score seed.
        moe_seed: u64,
        /// Dense score seed.
        dense_seed: u64,
    },
    /// A production score report had the wrong topology for §11.1.
    UnexpectedScoreTopology {
        /// Field that carried the unexpected topology.
        field: &'static str,
        /// Observed topology.
        observed: S7Topology,
        /// Expected topology.
        expected: S7Topology,
    },
}

impl fmt::Display for CollapseSweepError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LambdaSwitchNotInGrid { lambda_switch } => {
                write!(f, "lambda_switch {lambda_switch} is not in the D11 grid")
            }
            Self::UnexpectedSweepSeed { observed, expected } => write!(
                f,
                "collapse-sweep descriptor used seed {observed}, expected seed {expected}"
            ),
            Self::MissingBaseCheckpointHash => {
                f.write_str("collapse-sweep descriptor is missing base_checkpoint_sha")
            }
            Self::MissingValEvalSubsetHash => {
                f.write_str("collapse-sweep descriptor is missing val_eval_subset_sha")
            }
            Self::EmptyValEvalSubset => {
                f.write_str("collapse-sweep descriptor has an empty val_eval_subset")
            }
            Self::UnexpectedSweepExtraTrainSteps { observed, expected } => write!(
                f,
                "collapse-sweep descriptor requested {observed} extra train steps, expected {expected}"
            ),
            Self::UnexpectedLambdaSwitchGridHash { observed, expected } => write!(
                f,
                "collapse-sweep producer input used grid hash {observed}, expected {expected}"
            ),
            Self::UnexpectedGridCount { observed, expected } => {
                write!(
                    f,
                    "expected {expected} D11 grid entries, observed {observed}"
                )
            }
            Self::UnexpectedGridValue {
                index,
                observed,
                expected,
            } => write!(
                f,
                "D11 grid index {index} has lambda_switch {observed}, expected {expected}"
            ),
            Self::UnexpectedRecordCount { observed, expected } => {
                write!(
                    f,
                    "expected {expected} collapse-sweep records, observed {observed}"
                )
            }
            Self::DuplicateGridRecord { lambda_switch } => {
                write!(f, "duplicate collapse-sweep record for {lambda_switch}")
            }
            Self::MissingGridRecord { lambda_switch } => {
                write!(f, "missing collapse-sweep record for {lambda_switch}")
            }
            Self::RecordSeedMismatch {
                lambda_switch,
                expected,
                observed,
            } => write!(
                f,
                "lambda_switch {lambda_switch} has seed {observed}, expected {expected}"
            ),
            Self::BaseTrainStepMismatch {
                lambda_switch,
                expected,
                observed,
            } => write!(
                f,
                "lambda_switch {lambda_switch} has base train step {observed}, expected {expected}"
            ),
            Self::UnexpectedTrainingExtraStepDelta {
                lambda_switch,
                observed,
                expected,
            } => write!(
                f,
                "lambda_switch {lambda_switch} has training-extra step delta {observed}, expected {expected}"
            ),
            Self::TrainStepBeforeBase {
                base_train_step,
                train_step,
            } => write!(
                f,
                "collapse-sweep train step {train_step} precedes base step {base_train_step}"
            ),
            Self::TrainStepOverflow {
                base_train_step,
                extra_steps,
            } => write!(
                f,
                "collapse-sweep base step {base_train_step} plus {extra_steps} extra steps overflowed"
            ),
            Self::CompletedRecordMissingBpc { lambda_switch } => write!(
                f,
                "completed lambda_switch {lambda_switch} record is missing bpc_eval_subset"
            ),
            Self::CompletedRecordMissingQualityDelta { lambda_switch } => write!(
                f,
                "completed lambda_switch {lambda_switch} record is missing quality_delta_per_lambda_switch"
            ),
            Self::DivergedRecordHasBpc { lambda_switch } => write!(
                f,
                "diverged lambda_switch {lambda_switch} record must have null bpc_eval_subset"
            ),
            Self::DivergedRecordHasQualityDelta { lambda_switch } => write!(
                f,
                "diverged lambda_switch {lambda_switch} record must have null quality_delta_per_lambda_switch"
            ),
            Self::DivergenceStepOutOfRange {
                lambda_switch,
                base_train_step,
                train_step,
                divergence_step,
            } => write!(
                f,
                "lambda_switch {lambda_switch} divergence step {divergence_step} is outside ({base_train_step}, {train_step}]"
            ),
            Self::NonFiniteFloat { field, value } => {
                write!(f, "{field} must be finite, observed {value}")
            }
            Self::NegativeFloat { field, value } => {
                write!(f, "{field} must be nonnegative, observed {value}")
            }
            Self::EntropyBitsOutOfRange { value, max } => write!(
                f,
                "expert_usage_entropy_bits_mean must be in [0, {max}], observed {value}"
            ),
            Self::CanonicalJson { detail } => write!(f, "{detail}"),
            Self::CanonicalJsonUtf8 { detail } => {
                write!(f, "collapse-sweep canonical JSON was not UTF-8: {detail}")
            }
            Self::UnexpectedSchema { expected, observed } => write!(
                f,
                "unexpected collapse-sweep schema: expected {expected}, observed {observed}"
            ),
            Self::UnexpectedSchemaVersion { expected, observed } => write!(
                f,
                "unexpected collapse-sweep schema version: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "{field} mismatch: expected {expected}, observed {observed}"
            ),
            Self::UnexpectedProductionLambda { observed, expected } => write!(
                f,
                "production_lambda {observed} does not match D11/D5 expected {expected}"
            ),
            Self::UnexpectedCollapseThreshold { observed, expected } => write!(
                f,
                "collapse_threshold {observed} does not match D11 expected {expected}"
            ),
            Self::GuardrailVerdictMismatch { expected, observed } => write!(
                f,
                "guardrail_verdict mismatch: expected {expected:?}, observed {observed:?}"
            ),
            Self::ProductionSweepPointDidNotComplete => f.write_str(
                "production lambda sweep point did not complete; quality deltas are unavailable",
            ),
            Self::DegenerateSweepEmpty => {
                f.write_str("F8 constant-lambda sweep requires at least one record")
            }
            Self::DegenerateSweepNonProductionLambda { lambda_switch } => write!(
                f,
                "F8 constant-lambda sweep contained non-production lambda_switch {lambda_switch}"
            ),
            Self::ScoreSeedMismatch {
                moe_seed,
                dense_seed,
            } => write!(
                f,
                "production score seeds differ: MoE seed {moe_seed}, dense seed {dense_seed}"
            ),
            Self::UnexpectedScoreTopology {
                field,
                observed,
                expected,
            } => write!(
                f,
                "{field} has topology {observed:?}, expected {expected:?}"
            ),
        }
    }
}

impl std::error::Error for CollapseSweepError {}

impl From<CanonicalJsonError> for CollapseSweepError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson {
            detail: error.to_string(),
        }
    }
}

/// Canonicalize a lambda-switch value to the exact pinned D11 grid bit pattern.
pub fn canonicalize_d11_lambda_switch(lambda_switch: f32) -> Result<f32, CollapseSweepError> {
    let index = lambda_switch_grid_index(lambda_switch)?;
    Ok(D11_LAMBDA_SWITCH_GRID[index])
}

/// Canonicalize and validate the complete D11 lambda-switch grid in pinned order.
pub fn canonicalize_d11_lambda_switch_grid(grid: &[f32]) -> Result<Vec<f32>, CollapseSweepError> {
    if grid.len() != D11_LAMBDA_SWITCH_GRID.len() {
        return Err(CollapseSweepError::UnexpectedGridCount {
            observed: grid.len(),
            expected: D11_LAMBDA_SWITCH_GRID.len(),
        });
    }
    for (index, (observed, expected)) in grid
        .iter()
        .copied()
        .zip(D11_LAMBDA_SWITCH_GRID.iter().copied())
        .enumerate()
    {
        if observed.to_bits() != expected.to_bits() {
            return Err(CollapseSweepError::UnexpectedGridValue {
                index,
                observed,
                expected,
            });
        }
    }
    Ok(D11_LAMBDA_SWITCH_GRID.to_vec())
}

/// Hash the exact D11 lambda-switch grid bit patterns for deterministic replay.
pub fn lambda_switch_grid_hash(grid: &[f32]) -> Result<Hash256, CollapseSweepError> {
    let grid = canonicalize_d11_lambda_switch_grid(grid)?;
    let material = LambdaSwitchGridHashMaterial {
        schema: LAMBDA_SWITCH_GRID_SCHEMA,
        grid_bits: grid.iter().map(|lambda| lambda.to_bits()).collect(),
    };
    Ok(lambda_switch_grid_domain().hash(&material)?)
}

#[derive(Serialize)]
struct LambdaSwitchGridHashMaterial {
    schema: &'static str,
    grid_bits: Vec<u32>,
}

#[derive(Serialize)]
struct DeterministicFixturePointMaterial {
    schema: &'static str,
    purpose: &'static str,
    seed: u64,
    base_checkpoint_sha: Hash256,
    base_train_step: u64,
    val_eval_subset_sha: Hash256,
    val_eval_subset_len: u64,
    extra_train_steps: u64,
    lambda_switch_bits: u32,
    lambda_switch_grid_hash: Hash256,
}

fn deterministic_fixture_offset(
    input: LambdaSwitchSweepPointInput,
    purpose: &'static str,
    scale: f64,
) -> Result<f64, CollapseSweepError> {
    let material = DeterministicFixturePointMaterial {
        schema: FIXTURE_SWEEP_POINT_SCHEMA,
        purpose,
        seed: input.seed,
        base_checkpoint_sha: input.base_checkpoint_sha,
        base_train_step: input.base_train_step,
        val_eval_subset_sha: input.val_eval_subset_sha,
        val_eval_subset_len: input.val_eval_subset_len,
        extra_train_steps: input.extra_train_steps,
        lambda_switch_bits: input.lambda_switch.to_bits(),
        lambda_switch_grid_hash: input.lambda_switch_grid_hash,
    };
    let hash = deterministic_fixture_point_domain().hash(&material)?;
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hash.as_bytes()[..8]);
    let unit = u64::from_le_bytes(bytes) as f64 / u64::MAX as f64;
    Ok((unit - 0.5) * scale)
}

const fn lambda_switch_grid_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "D11LambdaSwitchGrid",
        LAMBDA_SWITCH_GRID_SCHEMA,
        LAMBDA_SWITCH_GRID_SCHEMA_VERSION_ID,
    )
}

const fn deterministic_fixture_point_domain() -> DomainHash<'static> {
    DomainHash::new(
        "gbf-experiments",
        "DeterministicFixtureSweepPoint",
        FIXTURE_SWEEP_POINT_SCHEMA,
        FIXTURE_SWEEP_POINT_SCHEMA_VERSION_ID,
    )
}

fn lambda_switch_grid_index(lambda_switch: f32) -> Result<usize, CollapseSweepError> {
    D11_LAMBDA_SWITCH_GRID
        .iter()
        .position(|candidate| candidate.to_bits() == lambda_switch.to_bits())
        .ok_or(CollapseSweepError::LambdaSwitchNotInGrid { lambda_switch })
}

fn required_record(
    records: &[LambdaSwitchSweepRecord],
    lambda_switch: f32,
) -> Result<&LambdaSwitchSweepRecord, CollapseSweepError> {
    records
        .iter()
        .find(|record| record.lambda_switch.to_bits() == lambda_switch.to_bits())
        .ok_or(CollapseSweepError::MissingGridRecord { lambda_switch })
}

fn validate_score_pair(
    production_moe_score: &S7ScoreReport,
    production_dense_matched_score: &S7ScoreReport,
) -> Result<(), CollapseSweepError> {
    if production_moe_score.seed != production_dense_matched_score.seed {
        return Err(CollapseSweepError::ScoreSeedMismatch {
            moe_seed: production_moe_score.seed,
            dense_seed: production_dense_matched_score.seed,
        });
    }
    if !matches!(&production_moe_score.topology, S7Topology::MoeTiny) {
        return Err(CollapseSweepError::UnexpectedScoreTopology {
            field: "production_moe_score.topology",
            observed: production_moe_score.topology.clone(),
            expected: S7Topology::MoeTiny,
        });
    }
    if !matches!(
        &production_dense_matched_score.topology,
        S7Topology::MoeTinyDenseMatched
    ) {
        return Err(CollapseSweepError::UnexpectedScoreTopology {
            field: "production_dense_matched_score.topology",
            observed: production_dense_matched_score.topology.clone(),
            expected: S7Topology::MoeTinyDenseMatched,
        });
    }
    Ok(())
}

fn validate_finite_nonnegative_f32(
    field: &'static str,
    value: f32,
) -> Result<(), CollapseSweepError> {
    validate_finite_nonnegative_f64(field, f64::from(value))
}

fn validate_finite_nonnegative_f64(
    field: &'static str,
    value: f64,
) -> Result<(), CollapseSweepError> {
    validate_finite_f64(field, value)?;
    if value < 0.0 {
        return Err(CollapseSweepError::NegativeFloat { field, value });
    }
    Ok(())
}

fn validate_finite_f64(field: &'static str, value: f64) -> Result<(), CollapseSweepError> {
    if !value.is_finite() {
        return Err(CollapseSweepError::NonFiniteFloat { field, value });
    }
    Ok(())
}

fn validate_entropy_bits_mean(value: f32) -> Result<(), CollapseSweepError> {
    let max = f32::from(S7_N_EXPERTS).log2();
    if value > max {
        return Err(CollapseSweepError::EntropyBitsOutOfRange { value, max });
    }
    Ok(())
}
