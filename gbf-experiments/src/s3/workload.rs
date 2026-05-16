//! S3 v0_success workload runner and manifest-adjacent helpers.

use std::error::Error;
use std::fmt;

use gbf_artifact::{DecodeMode, ModelArtifact, ReferenceModelBundle, TextCharSeq, is_text_char_id};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, canonical_json_bytes_omitting_fields,
    self_hash_omitting_fields,
};
use gbf_oracle::artifact::{ArtifactDecodeResult, ArtifactDecoder};
use gbf_oracle::scorers::{ArtifactScorer, ReferenceScorer};
use gbf_workload::{PromptId, WorkloadManifest_v0};
use serde::{Deserialize, Serialize};

use crate::s3::artifact::{ArtifactExportError, artifact_deployable_bytes};
use crate::s3::baseline::KnBaselineProduct;
use crate::s3::schema::{S3_V0_SUCCESS_SCHEMA, S3BuildKind};
use crate::s3::score::{BpcCharValue, S3_SCORE_CHUNK_SIZE, ScoreError, s3_score_bpc_char};

/// Tracing target for the v0_success runner and contamination helper.
pub const S3_WORKLOAD_LOG_TARGET: &str = "gbf_experiments::s3::workload";

/// v0_success run started event name.
pub const EVENT_NAME_V0_SUCCESS_RUN_STARTED: &str = "s3::v0_success::run_started";
/// v0_success seed started event name.
pub const EVENT_NAME_V0_SUCCESS_SEED_STARTED: &str = "s3::v0_success::seed_started";
/// v0_success per-prompt generation event name.
pub const EVENT_NAME_V0_SUCCESS_GENERATION_PER_PROMPT: &str =
    "s3::v0_success::generation_per_prompt";
/// v0_success scoring complete event name.
pub const EVENT_NAME_V0_SUCCESS_SCORING_COMPLETE: &str = "s3::v0_success::scoring_complete";
/// v0_success quality gate event name.
pub const EVENT_NAME_V0_SUCCESS_QUALITY_GATE: &str = "s3::v0_success::quality_gate";
/// v0_success run complete event name.
pub const EVENT_NAME_V0_SUCCESS_RUN_COMPLETE: &str = "s3::v0_success::run_complete";

/// Maximum characters generated per v0_success prompt.
pub const V0_SUCCESS_MAX_GENERATED_CHARS: usize = 256;
/// Minimum generated characters required by Q5.
pub const V0_SUCCESS_MIN_GENERATED_CHARS: u32 = 128;
/// Maximum allowed consecutive repeated token run for Q4.
pub const V0_SUCCESS_MAX_REPEAT: u32 = 8;
/// Minimum full-precision gain over Kneser-Ney for Q1.
pub const V0_SUCCESS_MIN_BPC_GAIN_VS_KN5: f64 = 0.05;
/// Maximum ternary-vs-full-precision bpc gap for Q2.
pub const V0_SUCCESS_MAX_QUANT_GAP: f64 = 0.5;
/// Suspicious-low median bpc sentinel from RFC §9.
pub const V0_SUCCESS_SUSPICIOUS_LOW_BPC_SENTINEL: f64 = 0.5;

const PRODUCT_SCHEMA_VERSION: &str = "1";

/// Conservative S3 chrome-budget slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RomBudgetSlot {
    /// Stable slot name.
    pub name: String,
    /// Synthetic default byte count.
    pub default_bytes: u64,
}

impl RomBudgetSlot {
    /// Construct a budget slot.
    #[must_use]
    pub fn new(name: impl Into<String>, default_bytes: u64) -> Self {
        Self {
            name: name.into(),
            default_bytes,
        }
    }
}

/// Conservative chrome budget used by S3 Q6.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConservativeChromeBudget {
    /// Synthetic ROM budget slots.
    pub slots: Vec<RomBudgetSlot>,
    /// Sum of `floor(0.90 * default_bytes)` over every slot.
    pub conservative_chrome_budget_bytes: u64,
    /// Self-hash over the budget record.
    pub chrome_budget_self_hash: Hash256,
}

impl ConservativeChromeBudget {
    /// Construct a budget and compute its self-hash.
    pub fn new(slots: Vec<RomBudgetSlot>) -> Result<Self, V0SuccessError> {
        if slots.is_empty() {
            return Err(V0SuccessError::EmptyChromeBudget);
        }
        let conservative_chrome_budget_bytes = conservative_chrome_budget_bytes(&slots);
        let mut budget = Self {
            slots,
            conservative_chrome_budget_bytes,
            chrome_budget_self_hash: Hash256::ZERO,
        };
        budget.chrome_budget_self_hash = budget.compute_self_hash()?;
        Ok(budget)
    }

    /// DomainHash context.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "ConservativeChromeBudget",
            "s3_chrome_budget.synthetic.v1",
            PRODUCT_SCHEMA_VERSION,
        )
    }

    /// Compute self-hash with `chrome_budget_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, V0SuccessError> {
        self_hash_omitting_fields(Self::domain(), self, "chrome_budget_self_hash", &[])
            .map_err(V0SuccessError::CanonicalJson)
    }
}

/// Compute the RFC D15 conservative chrome budget with exactly one 0.90 factor.
#[must_use]
pub fn conservative_chrome_budget_bytes(slots: &[RomBudgetSlot]) -> u64 {
    slots
        .iter()
        .map(|slot| (0.90_f64 * slot.default_bytes as f64).floor() as u64)
        .sum()
}

/// One decode step recorded in `s3_v0_success.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationDecodeStep {
    /// Zero-based decode step.
    pub step: u32,
    /// Argmax token selected at this step.
    pub token: u8,
    /// Maximum logit at this step.
    pub logit_max: f32,
}

/// Per-prompt generation record in `s3_v0_success.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationRecord {
    /// Prompt id.
    pub prompt_id: PromptId,
    /// Generated normalized text characters, excluding terminal EOS.
    pub generated_chars: TextCharSeq,
    /// Generated normalized text character count.
    pub generated_char_count: u32,
    /// Longest consecutive same-token run in `generated_chars`.
    pub max_consecutive_same_token: u32,
    /// Validity rate over generated text chars, excluding terminal EOS.
    pub charset_validity_rate: f64,
    /// Whether terminal EOS was observed before returning.
    pub terminal_eos_seen: bool,
    /// Decode mode used by v0_success.
    pub decode_mode: DecodeMode,
    /// Per-step argmax trace.
    pub decode_log: Vec<GenerationDecodeStep>,
}

impl GenerationRecord {
    /// Construct a checked generation record for tests and runner plumbing.
    pub fn new(
        prompt_id: impl Into<PromptId>,
        generated_chars: TextCharSeq,
        terminal_eos_seen: bool,
        decode_log: Vec<GenerationDecodeStep>,
        charset_validity_rate: f64,
    ) -> Result<Self, V0SuccessError> {
        if !(0.0..=1.0).contains(&charset_validity_rate) || !charset_validity_rate.is_finite() {
            return Err(V0SuccessError::InvalidCharsetValidityRate {
                value: charset_validity_rate,
            });
        }
        for step in &decode_log {
            if !step.logit_max.is_finite() {
                return Err(V0SuccessError::NonFiniteLogitMax { step: step.step });
            }
        }
        let generated_char_count = u32::try_from(generated_chars.len()).map_err(|_| {
            V0SuccessError::GeneratedCharCountOverflow {
                observed: generated_chars.len(),
            }
        })?;
        Ok(Self {
            prompt_id: prompt_id.into(),
            max_consecutive_same_token: max_consecutive_same_token(&generated_chars),
            generated_chars,
            generated_char_count,
            charset_validity_rate,
            terminal_eos_seen,
            decode_mode: DecodeMode::Argmax,
            decode_log,
        })
    }

    /// Convert an `ArtifactDecoder` result into the v0_success record shape.
    pub fn from_decode_result(
        prompt_id: impl Into<PromptId>,
        result: ArtifactDecodeResult,
    ) -> Result<Self, V0SuccessError> {
        let valid_count = result
            .generated
            .as_slice()
            .iter()
            .filter(|token| is_text_char_id(**token))
            .count();
        let charset_validity_rate = if result.generated.is_empty() {
            1.0
        } else {
            valid_count as f64 / result.generated.len() as f64
        };
        let decode_log = result
            .decode_log
            .into_iter()
            .map(|step| GenerationDecodeStep {
                step: step.step,
                token: step.token,
                logit_max: step.logit_max,
            })
            .collect();
        Self::new(
            prompt_id,
            result.generated,
            result.terminal_eos_seen,
            decode_log,
            charset_validity_rate,
        )
    }
}

/// Aggregated per-prompt generation metrics used by Q3/Q4/Q5.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerationAggregate {
    /// Weighted charset validity rate across all generated text chars.
    pub generated_token_charset_validity_rate: f64,
    /// Maximum same-token run over prompts.
    pub max_consecutive_same_token: u32,
    /// Minimum generated text count over prompts.
    pub min_generated_char_count: u32,
}

/// Aggregate v0_success per-prompt records according to RFC D6.
#[must_use]
pub fn aggregate_generation_records(records: &[GenerationRecord]) -> GenerationAggregate {
    let total_chars = records
        .iter()
        .map(|record| u64::from(record.generated_char_count))
        .sum::<u64>();
    let valid_chars = records
        .iter()
        .map(|record| record.charset_validity_rate * f64::from(record.generated_char_count))
        .sum::<f64>();
    let generated_token_charset_validity_rate = if total_chars == 0 {
        1.0
    } else {
        valid_chars / total_chars as f64
    };
    GenerationAggregate {
        generated_token_charset_validity_rate,
        max_consecutive_same_token: records
            .iter()
            .map(|record| record.max_consecutive_same_token)
            .max()
            .unwrap_or(0),
        min_generated_char_count: records
            .iter()
            .map(|record| record.generated_char_count)
            .min()
            .unwrap_or(0),
    }
}

/// Per-seed v0_success quality product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[allow(non_snake_case)]
pub struct V0SuccessPerSeed {
    /// S3 seed.
    pub seed: u64,
    /// Full-precision validation BPC-char.
    pub val_bpc_char_fp: BpcCharValue,
    /// Ternary artifact validation BPC-char.
    pub val_bpc_char_ternary: BpcCharValue,
    /// Full-precision gain over the Kneser-Ney baseline.
    pub bpc_gain_vs_kn5: f64,
    /// Ternary-vs-full-precision validation BPC-char gap.
    pub bpc_quant_gap: f64,
    /// Per-prompt generation quality records.
    pub per_prompt_generation: Vec<GenerationRecord>,
    /// Deployable artifact byte count.
    pub artifact_deployable_bytes: u64,
    /// Whether the artifact fits the conservative chrome budget.
    pub fits_chrome_budget: bool,
    /// Q1 quality gate result.
    pub Q1_holds: bool,
    /// Q2 quantization-gap gate result.
    pub Q2_holds: bool,
    /// Q3 charset-validity gate result.
    pub Q3_holds: bool,
    /// Q4 repeat-run gate result.
    pub Q4_holds: bool,
    /// Q5 generated-length gate result.
    pub Q5_holds: bool,
    /// Q6 chrome-budget gate result.
    pub Q6_holds: bool,
    /// Overall per-seed v0_success pass bit.
    pub pass: bool,
}

impl V0SuccessPerSeed {
    /// Construct a per-seed record from measured values and generation rows.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: u64,
        val_bpc_char_fp: BpcCharValue,
        val_bpc_char_ternary: BpcCharValue,
        bpc_kn5_baseline: f64,
        per_prompt_generation: Vec<GenerationRecord>,
        artifact_deployable_bytes: u64,
        conservative_chrome_budget_bytes: u64,
    ) -> Result<Self, V0SuccessError> {
        if per_prompt_generation.is_empty() {
            return Err(V0SuccessError::EmptyGenerationRecords { seed });
        }
        validate_generation_records(&per_prompt_generation)?;
        if !bpc_kn5_baseline.is_finite() || bpc_kn5_baseline < 0.0 {
            return Err(V0SuccessError::InvalidBaselineBpc {
                value: bpc_kn5_baseline,
            });
        }
        let bpc_gain_vs_kn5 = bpc_kn5_baseline - val_bpc_char_fp.get();
        let bpc_quant_gap = val_bpc_char_ternary.get() - val_bpc_char_fp.get();
        let generation = aggregate_generation_records(&per_prompt_generation);
        let fits_chrome_budget = artifact_deployable_bytes <= conservative_chrome_budget_bytes;
        let q1_holds = bpc_gain_vs_kn5 > V0_SUCCESS_MIN_BPC_GAIN_VS_KN5;
        let q2_holds = bpc_quant_gap <= V0_SUCCESS_MAX_QUANT_GAP;
        // Validity rates are bounded to [0, 1], so Q3 can require every
        // non-empty prompt to be exactly fully valid instead of applying an
        // epsilon to the weighted floating aggregate.
        let q3_holds = all_generated_chars_valid(&per_prompt_generation);
        let q4_holds = generation.max_consecutive_same_token <= V0_SUCCESS_MAX_REPEAT;
        let q5_holds = generation.min_generated_char_count >= V0_SUCCESS_MIN_GENERATED_CHARS;
        let q6_holds = fits_chrome_budget;
        Ok(Self::from_quality_bits_and_measurements(
            seed,
            val_bpc_char_fp,
            val_bpc_char_ternary,
            bpc_gain_vs_kn5,
            bpc_quant_gap,
            per_prompt_generation,
            artifact_deployable_bytes,
            fits_chrome_budget,
            q1_holds,
            q2_holds,
            q3_holds,
            q4_holds,
            q5_holds,
            q6_holds,
        ))
    }

    /// Construct a per-seed record with explicit Q bits for totality tests.
    #[allow(clippy::too_many_arguments)]
    pub fn from_quality_bits(
        seed: u64,
        q1_holds: bool,
        q2_holds: bool,
        q3_holds: bool,
        q4_holds: bool,
        q5_holds: bool,
        q6_holds: bool,
    ) -> Self {
        let generated_count = if q5_holds { 128 } else { 127 };
        let charset_validity_rate = if q3_holds { 1.0 } else { 0.99 };
        let max_repeat = if q4_holds { 8 } else { 9 };
        let record = GenerationRecord {
            prompt_id: PromptId::from(format!("prompt-{seed}")),
            generated_chars: TextCharSeq::new(vec![1; generated_count])
                .expect("quality-bit fixture chars validate"),
            generated_char_count: generated_count as u32,
            max_consecutive_same_token: max_repeat,
            charset_validity_rate,
            terminal_eos_seen: q5_holds,
            decode_mode: DecodeMode::Argmax,
            decode_log: Vec::new(),
        };
        Self::from_quality_bits_and_measurements(
            seed,
            BpcCharValue::try_new(if q1_holds { 1.0 } else { 2.0 }).expect("bpc"),
            BpcCharValue::try_new(if q2_holds { 1.1 } else { 2.7 }).expect("bpc"),
            if q1_holds { 0.1 } else { 0.0 },
            if q2_holds { 0.1 } else { 0.7 },
            vec![record],
            if q6_holds { 90 } else { 110 },
            q6_holds,
            q1_holds,
            q2_holds,
            q3_holds,
            q4_holds,
            q5_holds,
            q6_holds,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn from_quality_bits_and_measurements(
        seed: u64,
        val_bpc_char_fp: BpcCharValue,
        val_bpc_char_ternary: BpcCharValue,
        bpc_gain_vs_kn5: f64,
        bpc_quant_gap: f64,
        per_prompt_generation: Vec<GenerationRecord>,
        artifact_deployable_bytes: u64,
        fits_chrome_budget: bool,
        q1_holds: bool,
        q2_holds: bool,
        q3_holds: bool,
        q4_holds: bool,
        q5_holds: bool,
        q6_holds: bool,
    ) -> Self {
        let pass = q1_holds && q2_holds && q3_holds && q4_holds && q5_holds && q6_holds;
        Self {
            seed,
            val_bpc_char_fp,
            val_bpc_char_ternary,
            bpc_gain_vs_kn5,
            bpc_quant_gap,
            per_prompt_generation,
            artifact_deployable_bytes,
            fits_chrome_budget,
            Q1_holds: q1_holds,
            Q2_holds: q2_holds,
            Q3_holds: q3_holds,
            Q4_holds: q4_holds,
            Q5_holds: q5_holds,
            Q6_holds: q6_holds,
            pass,
        }
    }
}

/// Canonical `s3_v0_success.v1` product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct V0SuccessProduct {
    /// Pinned schema id.
    pub schema: String,
    /// Workload manifest self-hash.
    pub workload_self_hash: Hash256,
    /// KN baseline product self-hash.
    pub baseline_self_hash: Hash256,
    /// Conservative chrome budget self-hash.
    pub chrome_budget_self_hash: Hash256,
    /// Per-seed quality records.
    pub per_seed: Vec<V0SuccessPerSeed>,
    /// Suspicious-low median bpc sentinel.
    pub suspicious_low_bpc: bool,
    /// Overall pass iff all seeds pass and the suspicious sentinel is clear.
    pub overall_pass: bool,
    /// Self-hash over this product with this field omitted.
    pub v0_success_self_hash: Hash256,
}

impl V0SuccessProduct {
    /// Construct a product and compute its self-hash.
    pub fn new(
        workload_self_hash: Hash256,
        baseline_self_hash: Hash256,
        chrome_budget_self_hash: Hash256,
        per_seed: Vec<V0SuccessPerSeed>,
    ) -> Result<Self, V0SuccessError> {
        if per_seed.is_empty() {
            return Err(V0SuccessError::EmptyPerSeed);
        }
        let suspicious_low_bpc = median_bpc_fp(&per_seed) < V0_SUCCESS_SUSPICIOUS_LOW_BPC_SENTINEL;
        let overall_pass = per_seed.iter().all(|seed| seed.pass) && !suspicious_low_bpc;
        let mut product = Self {
            schema: S3_V0_SUCCESS_SCHEMA.to_owned(),
            workload_self_hash,
            baseline_self_hash,
            chrome_budget_self_hash,
            per_seed,
            suspicious_low_bpc,
            overall_pass,
            v0_success_self_hash: Hash256::ZERO,
        };
        product.v0_success_self_hash = product.compute_self_hash()?;
        Ok(product)
    }

    /// DomainHash context for `s3_v0_success.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "V0SuccessProduct",
            S3_V0_SUCCESS_SCHEMA,
            PRODUCT_SCHEMA_VERSION,
        )
    }

    /// Compute self-hash with `v0_success_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, V0SuccessError> {
        let bytes = canonical_json_bytes_omitting_fields(self, &["v0_success_self_hash"])
            .map_err(V0SuccessError::CanonicalJson)?;
        Self::domain()
            .hash_canonical_bytes(&bytes)
            .map_err(V0SuccessError::CanonicalJson)
    }

    /// Canonical JSON bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, V0SuccessError> {
        CanonicalJson::to_vec(self).map_err(V0SuccessError::CanonicalJson)
    }
}

/// Run the S3 v0_success workload contract.
#[must_use]
pub fn s3_run_v0_success(
    artifacts: Vec<ModelArtifact>,
    bundles: Vec<ReferenceModelBundle>,
    workload: WorkloadManifest_v0,
    val_post: TextCharSeq,
    baseline: KnBaselineProduct,
    chrome_budget: ConservativeChromeBudget,
) -> V0SuccessProduct {
    try_s3_run_v0_success(
        artifacts,
        bundles,
        workload,
        val_post,
        baseline,
        chrome_budget,
    )
    .expect("v0_success inputs must satisfy the S3 runner contract")
}

/// Checked form of [`s3_run_v0_success`].
pub fn try_s3_run_v0_success(
    artifacts: Vec<ModelArtifact>,
    bundles: Vec<ReferenceModelBundle>,
    workload: WorkloadManifest_v0,
    val_post: TextCharSeq,
    baseline: KnBaselineProduct,
    chrome_budget: ConservativeChromeBudget,
) -> Result<V0SuccessProduct, V0SuccessError> {
    if artifacts.len() != workload.seeds.len() {
        return Err(V0SuccessError::SeedArtifactArityMismatch {
            seeds: workload.seeds.len(),
            artifacts: artifacts.len(),
        });
    }
    if bundles.len() != workload.seeds.len() {
        return Err(V0SuccessError::SeedBundleArityMismatch {
            seeds: workload.seeds.len(),
            bundles: bundles.len(),
        });
    }

    tracing::info!(
        target: S3_WORKLOAD_LOG_TARGET,
        event_name = EVENT_NAME_V0_SUCCESS_RUN_STARTED,
        seed_count = workload.seeds.len() as u64,
        prompt_count = workload.prompts.len() as u64,
        workload_self_hash = %workload.workload_self_hash,
    );

    let build_kind = active_build_kind()?;
    let mut per_seed = Vec::new();
    for ((seed, artifact), bundle) in workload
        .seeds
        .iter()
        .copied()
        .zip(artifacts.iter())
        .zip(bundles.iter())
    {
        tracing::info!(
            target: S3_WORKLOAD_LOG_TARGET,
            event_name = EVENT_NAME_V0_SUCCESS_SEED_STARTED,
            seed,
            build_kind = build_kind.as_str(),
        );

        let decoder = ArtifactDecoder::new(artifact);
        let mut per_prompt_generation = Vec::new();
        for prompt in &workload.prompts {
            let decoded =
                decoder.decode_argmax(&prompt.prompt_chars, V0_SUCCESS_MAX_GENERATED_CHARS, true);
            let record = GenerationRecord::from_decode_result(prompt.id.clone(), decoded)?;
            tracing::trace!(
                target: S3_WORKLOAD_LOG_TARGET,
                event_name = EVENT_NAME_V0_SUCCESS_GENERATION_PER_PROMPT,
                seed,
                prompt_id = prompt.id.as_str(),
                generated_char_count = record.generated_char_count as u64,
                terminal_eos_seen = record.terminal_eos_seen,
                max_consecutive_same_token = record.max_consecutive_same_token as u64,
                charset_validity_rate = record.charset_validity_rate,
            );
            per_prompt_generation.push(record);
        }

        let val_bpc_char_fp =
            s3_score_bpc_char(ReferenceScorer::new(bundle), &val_post, S3_SCORE_CHUNK_SIZE)
                .bpc_char;
        let val_bpc_char_ternary = s3_score_bpc_char(
            ArtifactScorer::new(artifact),
            &val_post,
            S3_SCORE_CHUNK_SIZE,
        )
        .bpc_char;
        let artifact_deployable_bytes = artifact_deployable_bytes(artifact)?;
        let per_seed_record = V0SuccessPerSeed::new(
            seed,
            val_bpc_char_fp,
            val_bpc_char_ternary,
            baseline.bpc_kn5_val,
            per_prompt_generation,
            artifact_deployable_bytes,
            chrome_budget.conservative_chrome_budget_bytes,
        )?;

        tracing::info!(
            target: S3_WORKLOAD_LOG_TARGET,
            event_name = EVENT_NAME_V0_SUCCESS_SCORING_COMPLETE,
            seed,
            val_bpc_char_fp = per_seed_record.val_bpc_char_fp.get(),
            val_bpc_char_ternary = per_seed_record.val_bpc_char_ternary.get(),
            bpc_gain_vs_kn5 = per_seed_record.bpc_gain_vs_kn5,
            bpc_quant_gap = per_seed_record.bpc_quant_gap,
            artifact_deployable_bytes = per_seed_record.artifact_deployable_bytes,
        );
        tracing::info!(
            target: S3_WORKLOAD_LOG_TARGET,
            event_name = EVENT_NAME_V0_SUCCESS_QUALITY_GATE,
            seed,
            Q1_holds = per_seed_record.Q1_holds,
            Q2_holds = per_seed_record.Q2_holds,
            Q3_holds = per_seed_record.Q3_holds,
            Q4_holds = per_seed_record.Q4_holds,
            Q5_holds = per_seed_record.Q5_holds,
            Q6_holds = per_seed_record.Q6_holds,
            pass = per_seed_record.pass,
        );
        per_seed.push(per_seed_record);
    }

    let product = V0SuccessProduct::new(
        workload.workload_self_hash,
        baseline.baseline_self_hash,
        chrome_budget.chrome_budget_self_hash,
        per_seed,
    )?;
    tracing::info!(
        target: S3_WORKLOAD_LOG_TARGET,
        event_name = EVENT_NAME_V0_SUCCESS_RUN_COMPLETE,
        seed_count = product.per_seed.len() as u64,
        overall_pass = product.overall_pass,
        suspicious_low_bpc = product.suspicious_low_bpc,
        v0_success_self_hash = %product.v0_success_self_hash,
    );
    Ok(product)
}

fn active_build_kind() -> Result<S3BuildKind, V0SuccessError> {
    if cfg!(feature = "s3-oracle-fallback") {
        Ok(S3BuildKind::s3_v0_success_fallback_oracle)
    } else if cfg!(feature = "s3-oracle-real") {
        Ok(S3BuildKind::s3_v0_success_real_oracle)
    } else {
        Err(V0SuccessError::OracleBackendFeatureDisabled)
    }
}

fn validate_generation_records(records: &[GenerationRecord]) -> Result<(), V0SuccessError> {
    for record in records {
        if !(0.0..=1.0).contains(&record.charset_validity_rate)
            || !record.charset_validity_rate.is_finite()
        {
            return Err(V0SuccessError::InvalidCharsetValidityRate {
                value: record.charset_validity_rate,
            });
        }
        for step in &record.decode_log {
            if !step.logit_max.is_finite() {
                return Err(V0SuccessError::NonFiniteLogitMax { step: step.step });
            }
        }
    }
    Ok(())
}

fn all_generated_chars_valid(records: &[GenerationRecord]) -> bool {
    records
        .iter()
        .all(|record| record.generated_char_count == 0 || record.charset_validity_rate == 1.0)
}

fn median_bpc_fp(per_seed: &[V0SuccessPerSeed]) -> f64 {
    let mut values = per_seed
        .iter()
        .map(|seed| seed.val_bpc_char_fp.get())
        .collect::<Vec<_>>();
    values.sort_by(|left, right| left.total_cmp(right));
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn max_consecutive_same_token(chars: &TextCharSeq) -> u32 {
    let mut max_run = 0_u32;
    let mut current_run = 0_u32;
    let mut previous = None;
    for token in chars.as_slice() {
        if Some(*token) == previous {
            current_run += 1;
        } else {
            current_run = 1;
            previous = Some(*token);
        }
        max_run = max_run.max(current_run);
    }
    max_run
}

/// Errors from the v0_success runner.
#[derive(Debug)]
pub enum V0SuccessError {
    /// Number of artifacts did not match the workload seed matrix.
    SeedArtifactArityMismatch {
        /// Seed count.
        seeds: usize,
        /// Artifact count.
        artifacts: usize,
    },
    /// Number of bundles did not match the workload seed matrix.
    SeedBundleArityMismatch {
        /// Seed count.
        seeds: usize,
        /// Bundle count.
        bundles: usize,
    },
    /// The v0_success runner requires an explicit S3 oracle backend feature.
    OracleBackendFeatureDisabled,
    /// Chrome budget must contain at least one slot.
    EmptyChromeBudget,
    /// Product must contain at least one seed.
    EmptyPerSeed,
    /// Per-seed record must contain prompt generations.
    EmptyGenerationRecords {
        /// Seed.
        seed: u64,
    },
    /// Baseline BPC was invalid.
    InvalidBaselineBpc {
        /// Observed value.
        value: f64,
    },
    /// Charset validity rate was invalid.
    InvalidCharsetValidityRate {
        /// Observed value.
        value: f64,
    },
    /// A decode step had a non-finite max logit.
    NonFiniteLogitMax {
        /// Decode step.
        step: u32,
    },
    /// Generated length exceeded u32.
    GeneratedCharCountOverflow {
        /// Observed length.
        observed: usize,
    },
    /// Artifact deployable byte computation failed.
    ArtifactExport(ArtifactExportError),
    /// Scoring failed.
    Score(ScoreError),
    /// Canonical JSON failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for V0SuccessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SeedArtifactArityMismatch { seeds, artifacts } => {
                write!(f, "workload has {seeds} seeds but {artifacts} artifacts")
            }
            Self::SeedBundleArityMismatch { seeds, bundles } => {
                write!(f, "workload has {seeds} seeds but {bundles} bundles")
            }
            Self::OracleBackendFeatureDisabled => {
                f.write_str("v0_success requires s3-oracle-real or s3-oracle-fallback")
            }
            Self::EmptyChromeBudget => f.write_str("conservative chrome budget has no slots"),
            Self::EmptyPerSeed => f.write_str("v0_success product has no per-seed records"),
            Self::EmptyGenerationRecords { seed } => {
                write!(f, "seed {seed} has no generation records")
            }
            Self::InvalidBaselineBpc { value } => {
                write!(
                    f,
                    "baseline bpc_kn5_val must be finite and non-negative, got {value}"
                )
            }
            Self::InvalidCharsetValidityRate { value } => {
                write!(
                    f,
                    "charset validity rate must be finite in [0, 1], got {value}"
                )
            }
            Self::NonFiniteLogitMax { step } => {
                write!(f, "decode step {step} has non-finite logit_max")
            }
            Self::GeneratedCharCountOverflow { observed } => {
                write!(f, "generated char count {observed} does not fit u32")
            }
            Self::ArtifactExport(error) => write!(f, "{error}"),
            Self::Score(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for V0SuccessError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::ArtifactExport(error) => Some(error),
            Self::Score(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::SeedArtifactArityMismatch { .. }
            | Self::SeedBundleArityMismatch { .. }
            | Self::OracleBackendFeatureDisabled
            | Self::EmptyChromeBudget
            | Self::EmptyPerSeed
            | Self::EmptyGenerationRecords { .. }
            | Self::InvalidBaselineBpc { .. }
            | Self::InvalidCharsetValidityRate { .. }
            | Self::NonFiniteLogitMax { .. }
            | Self::GeneratedCharCountOverflow { .. } => None,
        }
    }
}

impl From<ArtifactExportError> for V0SuccessError {
    fn from(error: ArtifactExportError) -> Self {
        Self::ArtifactExport(error)
    }
}

impl From<ScoreError> for V0SuccessError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}
