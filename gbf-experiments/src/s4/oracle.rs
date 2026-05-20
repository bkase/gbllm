//! S4 oracle-agreement surface.

use std::error::Error;
use std::fmt;

use gbf_artifact::{ModelArtifact, ReferenceModelBundle, TextCharSeq, VOCAB_SIZE};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, canonical_json_bytes_omitting_fields,
    sha256,
};
use gbf_oracle::scorers::{ArtifactScorer, ReferenceScorer};
use gbf_workload::AcceptanceMatrix_S3;
use serde::{Deserialize, Serialize};

use crate::s3::score::{Evaluator, EvaluatorOutput, S3_SCORE_CHUNK_SIZE, ScoreError};
use crate::s4::schema::{S4SchemaError, validate_s4_seed};

/// Schema id for the seed-0 Gutenberg oracle agreement artifact.
pub const S4_ORACLE_AGREEMENT_SCHEMA: &str = "s4_oracle_agreement.v1";

/// Schema id for the S3-inherited tolerance hash payload.
pub const S4_S3_ORACLE_TOLERANCE_SCHEMA: &str = "s3_oracle_agreement_tolerances.v1";

/// S4 oracle agreement tracing target.
pub const S4_ORACLE_LOG_TARGET: &str = "gbf_experiments::s4::oracle";

/// S4 D15 only gates seed 0 in v1.
pub const S4_ORACLE_MANDATORY_SEED: u64 = 0;

/// Structured event emitted when S4 oracle agreement starts.
pub const S4_ORACLE_AGREEMENT_STARTED_EVENT_NAME: &str = "s4_oracle_agreement_started";

/// Structured per-token scoring event emitted by S4 oracle agreement.
pub const S4_ORACLE_SCORE_EVENT_NAME: &str = "s4_oracle_score";

/// Structured event emitted when S4 oracle agreement finalizes.
pub const S4_ORACLE_AGREEMENT_FINALIZED_EVENT_NAME: &str = "s4_oracle_agreement_finalized";

const S4_ORACLE_AGREEMENT_SCHEMA_VERSION: &str = "1";
const S4_ORACLE_AGREEMENT_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4OracleAgreementReport",
    S4_ORACLE_AGREEMENT_SCHEMA,
    S4_ORACLE_AGREEMENT_SCHEMA_VERSION,
);
const S4_S3_ORACLE_TOLERANCE_DOMAIN: DomainHash<'static> = DomainHash::new(
    "gbf-experiments",
    "S4InheritedS3OracleTolerances",
    S4_S3_ORACLE_TOLERANCE_SCHEMA,
    S4_ORACLE_AGREEMENT_SCHEMA_VERSION,
);
const LOG2_E: f64 = std::f64::consts::LOG2_E;

/// BPC-gap tolerances inherited from the S3 oracle agreement matrix.
///
/// The source S3 fields are `max_per_token_logit_abs_diff` scalars. D15 reuses
/// those pinned scalar budgets for S4's per-token bpc gap comparisons; the
/// fields below are named for the S4 comparison unit they gate. S3 does not
/// carry a direct ReferenceModelBundle-vs-ArtifactOracle gate, so the inherited
/// inter-oracle budget is `max(phase_a, phase_d)`: the least surprising bound
/// that preserves both S3 pairwise gates without introducing a new tolerance.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4InheritedS3OracleTolerances {
    /// Schema id for this hash payload.
    pub schema: String,
    /// Live-training vs ReferenceModelBundle per-token bpc-gap tolerance.
    pub live_vs_denotational_bpc: f64,
    /// Live-training vs ArtifactOracle per-token bpc-gap tolerance.
    pub live_vs_artifact_bpc: f64,
    /// ReferenceModelBundle vs ArtifactOracle per-token bpc-gap tolerance.
    pub denotational_vs_artifact_bpc: f64,
}

impl S4InheritedS3OracleTolerances {
    /// Recompute the S4 D15 tolerance payload from S3's pinned agreement gates.
    pub fn s3_pinned() -> Result<Self, S4OracleAgreementError> {
        let acceptance = AcceptanceMatrix_S3::pinned();
        let phase_a = acceptance
            .live_phase_a_vs_bundle
            .ok_or(S4OracleAgreementError::MissingS3Tolerance {
                field: "live_phase_a_vs_bundle",
            })?
            .max_per_token_logit_abs_diff;
        let phase_d = acceptance
            .live_phase_d_vs_artifact
            .ok_or(S4OracleAgreementError::MissingS3Tolerance {
                field: "live_phase_d_vs_artifact",
            })?
            .max_per_token_logit_abs_diff;
        let tolerances = Self {
            schema: S4_S3_ORACLE_TOLERANCE_SCHEMA.to_owned(),
            live_vs_denotational_bpc: phase_a,
            live_vs_artifact_bpc: phase_d,
            denotational_vs_artifact_bpc: phase_a.max(phase_d),
        };
        tolerances.validate()?;
        Ok(tolerances)
    }

    /// Compute the pinned S3 tolerance self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4OracleAgreementError> {
        self.validate()?;
        S4_S3_ORACLE_TOLERANCE_DOMAIN
            .hash(self)
            .map_err(S4OracleAgreementError::CanonicalJson)
    }

    fn validate(&self) -> Result<(), S4OracleAgreementError> {
        if self.schema != S4_S3_ORACLE_TOLERANCE_SCHEMA {
            return Err(S4OracleAgreementError::InvalidToleranceSchema {
                observed: self.schema.clone(),
            });
        }
        validate_finite_nonnegative("live_vs_denotational_bpc", self.live_vs_denotational_bpc)?;
        validate_finite_nonnegative("live_vs_artifact_bpc", self.live_vs_artifact_bpc)?;
        validate_finite_nonnegative(
            "denotational_vs_artifact_bpc",
            self.denotational_vs_artifact_bpc,
        )?;
        Ok(())
    }
}

/// Inputs for generic three-way Gutenberg oracle agreement scoring.
pub struct S4OracleAgreementInputs<'a, L, R, A>
where
    L: Evaluator,
    R: Evaluator,
    A: Evaluator,
{
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// S4 seed. D15 requires seed 0 for the mandatory report.
    pub seed: u64,
    /// Gutenberg checkpoint self-hash.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation token stream.
    pub corpus_val_sha: Hash256,
    /// Expected workload manifest self-hash bound by the caller.
    pub expected_workload_manifest_self_hash: Hash256,
    /// Observed workload manifest self-hash attached to the scoring inputs.
    pub workload_manifest_self_hash: Hash256,
    /// Self-hash of the S3-pinned fixture set evaluated on Gutenberg val.
    pub fixture_set_self_hash: Hash256,
    /// Normalized Gutenberg validation token stream.
    pub gutenberg_val: &'a TextCharSeq,
    /// Live training scorer.
    pub live_training_scorer: L,
    /// ReferenceModelBundle re-export scorer.
    pub reference_model_bundle_scorer: R,
    /// ArtifactOracle scorer.
    pub artifact_oracle_scorer: A,
}

/// Inputs for the concrete ReferenceModelBundle + ModelArtifact scorer wrapper.
pub struct S4BundleArtifactOracleAgreementInputs<'a, L>
where
    L: Evaluator,
{
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// S4 seed. D15 requires seed 0 for the mandatory report.
    pub seed: u64,
    /// Gutenberg checkpoint self-hash.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation token stream.
    pub corpus_val_sha: Hash256,
    /// Expected workload manifest self-hash bound by the caller.
    pub expected_workload_manifest_self_hash: Hash256,
    /// Observed workload manifest self-hash attached to the scoring inputs.
    pub workload_manifest_self_hash: Hash256,
    /// Self-hash of the S3-pinned fixture set evaluated on Gutenberg val.
    pub fixture_set_self_hash: Hash256,
    /// Normalized Gutenberg validation token stream.
    pub gutenberg_val: &'a TextCharSeq,
    /// Live training scorer.
    pub live_training_scorer: L,
    /// ReferenceModelBundle re-export over the Gutenberg checkpoint.
    pub reference_model_bundle: &'a ReferenceModelBundle,
    /// Artifact evaluated through the ArtifactOracle scorer.
    pub artifact: &'a ModelArtifact,
}

/// Per-token row recorded in `s4_oracle_agreement.v1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4OracleAgreementTokenRecord {
    /// Zero-based token index in `gutenberg_val`.
    pub token: u64,
    /// Target charset-v1 id at this position.
    pub target_token_id: u8,
    /// Live training per-token bpc.
    pub bpc_live: f64,
    /// ReferenceModelBundle per-token bpc.
    pub bpc_denotational: f64,
    /// ArtifactOracle per-token bpc.
    pub bpc_artifact: f64,
    /// Absolute live-vs-denotational bpc gap.
    pub gap_live_vs_denotational: f64,
    /// Absolute live-vs-artifact bpc gap.
    pub gap_live_vs_artifact: f64,
    /// Absolute denotational-vs-artifact bpc gap.
    pub gap_denotational_vs_artifact: f64,
}

/// Overall S4 oracle agreement outcome.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase", deny_unknown_fields)]
pub enum S4OracleAgreementOutcome {
    /// All token gaps were inside S3-pinned tolerances.
    Agree,
    /// At least one token exceeded an inherited S3 tolerance.
    Disagree {
        /// First token whose three-way max gap exceeded tolerance.
        failing_token: u64,
        /// Largest pairwise bpc gap observed on that token.
        max_gap: f64,
    },
}

impl S4OracleAgreementOutcome {
    /// Stable tracing/report label for the outcome variant.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Agree => "Agree",
            Self::Disagree { .. } => "Disagree",
        }
    }
}

/// `s4_oracle_agreement.v1` report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct S4OracleAgreementReport {
    /// Schema id, always `s4_oracle_agreement.v1`.
    pub schema: String,
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// Mandatory D15 seed, currently `0`.
    pub seed: u64,
    /// Gutenberg checkpoint self-hash.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation token stream.
    pub corpus_val_sha: Hash256,
    /// Workload manifest self-hash bound to the scoring inputs.
    pub workload_manifest_self_hash: Hash256,
    /// Self-hash of the S3-pinned fixture set evaluated on Gutenberg val.
    pub fixture_set_self_hash: Hash256,
    /// Mean live-training bpc over the token records.
    pub bpc_live: f64,
    /// Mean ReferenceModelBundle bpc over the token records.
    pub bpc_denotational: f64,
    /// Mean ArtifactOracle bpc over the token records.
    pub bpc_artifact: f64,
    /// Maximum live-vs-denotational per-token bpc gap.
    pub gap_live_vs_denotational: f64,
    /// Maximum live-vs-artifact per-token bpc gap.
    pub gap_live_vs_artifact: f64,
    /// Maximum denotational-vs-artifact per-token bpc gap.
    pub gap_denotational_vs_artifact: f64,
    /// Per-token BPC streams and gaps.
    pub per_token: Vec<S4OracleAgreementTokenRecord>,
    /// Self-hash of the S3-derived tolerance payload.
    pub s3_tolerance_self_hash: Hash256,
    /// Overall agreement outcome.
    pub outcome: S4OracleAgreementOutcome,
    /// Self-hash over canonical JSON with this field omitted.
    pub oracle_agreement_self_hash: Hash256,
}

impl S4OracleAgreementReport {
    /// Compute the report self-hash with `oracle_agreement_self_hash` omitted.
    pub fn compute_self_hash(&self) -> Result<Hash256, S4OracleAgreementError> {
        let bytes = canonical_json_bytes_omitting_fields(self, &["oracle_agreement_self_hash"])?;
        S4_ORACLE_AGREEMENT_DOMAIN
            .hash_canonical_bytes(&bytes)
            .map_err(S4OracleAgreementError::CanonicalJson)
    }

    /// Canonical JSON bytes including `oracle_agreement_self_hash`.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, S4OracleAgreementError> {
        self.validate_canonical_write()?;
        CanonicalJson::to_vec(self).map_err(S4OracleAgreementError::CanonicalJson)
    }

    /// Validate structure and self-hash.
    pub fn validate_canonical_write(&self) -> Result<(), S4OracleAgreementError> {
        self.validate_structure()?;
        let recomputed = self.compute_self_hash()?;
        if recomputed != self.oracle_agreement_self_hash {
            return Err(S4OracleAgreementError::SelfHashMismatch {
                expected: recomputed,
                observed: self.oracle_agreement_self_hash,
            });
        }
        Ok(())
    }

    fn validate_structure(&self) -> Result<(), S4OracleAgreementError> {
        if self.schema != S4_ORACLE_AGREEMENT_SCHEMA {
            return Err(S4OracleAgreementError::InvalidSchema {
                observed: self.schema.clone(),
            });
        }
        validate_mandatory_seed(self.seed)?;
        validate_nonzero_hash(
            "tinystories_manifest_self_hash",
            self.tinystories_manifest_self_hash,
        )?;
        validate_nonzero_hash(
            "gutenberg_manifest_self_hash",
            self.gutenberg_manifest_self_hash,
        )?;
        validate_nonzero_hash("checkpoint_self_hash", self.checkpoint_self_hash)?;
        validate_nonzero_hash("corpus_val_sha", self.corpus_val_sha)?;
        validate_nonzero_hash(
            "workload_manifest_self_hash",
            self.workload_manifest_self_hash,
        )?;
        validate_nonzero_hash("fixture_set_self_hash", self.fixture_set_self_hash)?;
        validate_nonzero_hash("s3_tolerance_self_hash", self.s3_tolerance_self_hash)?;
        if self.per_token.is_empty() {
            return Err(S4OracleAgreementError::EmptyValidation);
        }
        validate_finite_nonnegative("bpc_live", self.bpc_live)?;
        validate_finite_nonnegative("bpc_denotational", self.bpc_denotational)?;
        validate_finite_nonnegative("bpc_artifact", self.bpc_artifact)?;
        validate_finite_nonnegative("gap_live_vs_denotational", self.gap_live_vs_denotational)?;
        validate_finite_nonnegative("gap_live_vs_artifact", self.gap_live_vs_artifact)?;
        validate_finite_nonnegative(
            "gap_denotational_vs_artifact",
            self.gap_denotational_vs_artifact,
        )?;

        let tolerances = S4InheritedS3OracleTolerances::s3_pinned()?;
        let tolerance_hash = tolerances.compute_self_hash()?;
        if self.s3_tolerance_self_hash != tolerance_hash {
            return Err(S4OracleAgreementError::ToleranceHashMismatch {
                expected: tolerance_hash,
                observed: self.s3_tolerance_self_hash,
            });
        }

        let recomputed = summarize_records(&self.per_token, &tolerances)?;
        if !same_f64_bits(self.bpc_live, recomputed.bpc_live)
            || !same_f64_bits(self.bpc_denotational, recomputed.bpc_denotational)
            || !same_f64_bits(self.bpc_artifact, recomputed.bpc_artifact)
            || !same_f64_bits(
                self.gap_live_vs_denotational,
                recomputed.gap_live_vs_denotational,
            )
            || !same_f64_bits(self.gap_live_vs_artifact, recomputed.gap_live_vs_artifact)
            || !same_f64_bits(
                self.gap_denotational_vs_artifact,
                recomputed.gap_denotational_vs_artifact,
            )
            || self.outcome != recomputed.outcome
        {
            return Err(S4OracleAgreementError::SummaryMismatch);
        }
        Ok(())
    }
}

/// Run S4 D15 oracle agreement with concrete bundle and artifact scorers.
pub fn s4_oracle_agreement_for_bundle_and_artifact<L>(
    inputs: S4BundleArtifactOracleAgreementInputs<'_, L>,
) -> Result<S4OracleAgreementReport, S4OracleAgreementError>
where
    L: Evaluator,
{
    s4_oracle_agreement(S4OracleAgreementInputs {
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        seed: inputs.seed,
        checkpoint_self_hash: inputs.checkpoint_self_hash,
        corpus_val_sha: inputs.corpus_val_sha,
        expected_workload_manifest_self_hash: inputs.expected_workload_manifest_self_hash,
        workload_manifest_self_hash: inputs.workload_manifest_self_hash,
        fixture_set_self_hash: inputs.fixture_set_self_hash,
        gutenberg_val: inputs.gutenberg_val,
        live_training_scorer: inputs.live_training_scorer,
        reference_model_bundle_scorer: ReferenceScorer::new(inputs.reference_model_bundle),
        artifact_oracle_scorer: ArtifactScorer::new(inputs.artifact),
    })
}

/// Run S4 D15 oracle agreement with caller-supplied scorer implementations.
pub fn s4_oracle_agreement<L, R, A>(
    inputs: S4OracleAgreementInputs<'_, L, R, A>,
) -> Result<S4OracleAgreementReport, S4OracleAgreementError>
where
    L: Evaluator,
    R: Evaluator,
    A: Evaluator,
{
    validate_binding(&inputs)?;
    tracing::info!(
        target: S4_ORACLE_LOG_TARGET,
        event_name = S4_ORACLE_AGREEMENT_STARTED_EVENT_NAME,
        schema = S4_ORACLE_AGREEMENT_SCHEMA,
        tinystories_manifest_self_hash = %inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash = %inputs.gutenberg_manifest_self_hash,
        workload_manifest_self_hash = %inputs.workload_manifest_self_hash,
        expected_workload_manifest_self_hash = %inputs.expected_workload_manifest_self_hash,
        checkpoint_self_hash = %inputs.checkpoint_self_hash,
        corpus_val_sha = %inputs.corpus_val_sha,
        fixture_set_self_hash = %inputs.fixture_set_self_hash,
        seed = inputs.seed,
        token_count = inputs.gutenberg_val.len() as u64,
        "s4 oracle agreement started"
    );

    let live = score_per_token_stream(
        "live_training",
        inputs.live_training_scorer,
        inputs.gutenberg_val,
    )?;
    let denotational = score_per_token_stream(
        "reference_model_bundle",
        inputs.reference_model_bundle_scorer,
        inputs.gutenberg_val,
    )?;
    let artifact = score_per_token_stream(
        "artifact_oracle",
        inputs.artifact_oracle_scorer,
        inputs.gutenberg_val,
    )?;

    let report = s4_oracle_agreement_from_streams(S4OracleAgreementStreamInputs {
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        seed: inputs.seed,
        checkpoint_self_hash: inputs.checkpoint_self_hash,
        corpus_val_sha: inputs.corpus_val_sha,
        workload_manifest_self_hash: inputs.workload_manifest_self_hash,
        fixture_set_self_hash: inputs.fixture_set_self_hash,
        live,
        denotational,
        artifact,
    })?;

    tracing::info!(
        target: S4_ORACLE_LOG_TARGET,
        event_name = S4_ORACLE_AGREEMENT_FINALIZED_EVENT_NAME,
        schema = report.schema.as_str(),
        seed = report.seed,
        token_count = report.per_token.len() as u64,
        outcome = report.outcome.as_str(),
        gap_live_vs_denotational = report.gap_live_vs_denotational,
        gap_live_vs_artifact = report.gap_live_vs_artifact,
        gap_denotational_vs_artifact = report.gap_denotational_vs_artifact,
        s3_tolerance_self_hash = %report.s3_tolerance_self_hash,
        oracle_agreement_self_hash = %report.oracle_agreement_self_hash,
        "s4 oracle agreement finalized"
    );
    Ok(report)
}

/// Pre-scored inputs used by tests and by callers that already own scoring.
#[derive(Debug, Clone, PartialEq)]
pub struct S4OracleAgreementStreamInputs {
    /// TinyStories manifest self-hash carried for lineage.
    pub tinystories_manifest_self_hash: Hash256,
    /// Gutenberg manifest self-hash.
    pub gutenberg_manifest_self_hash: Hash256,
    /// S4 seed. D15 requires seed 0 for the mandatory report.
    pub seed: u64,
    /// Gutenberg checkpoint self-hash.
    pub checkpoint_self_hash: Hash256,
    /// SHA-256 of the Gutenberg validation token stream.
    pub corpus_val_sha: Hash256,
    /// Workload manifest self-hash bound to the scoring inputs.
    pub workload_manifest_self_hash: Hash256,
    /// Self-hash of the S3-pinned fixture set evaluated on Gutenberg val.
    pub fixture_set_self_hash: Hash256,
    /// Live training per-token bpc stream.
    pub live: Vec<S4OracleTokenBpc>,
    /// ReferenceModelBundle per-token bpc stream.
    pub denotational: Vec<S4OracleTokenBpc>,
    /// ArtifactOracle per-token bpc stream.
    pub artifact: Vec<S4OracleTokenBpc>,
}

/// One per-token bpc value from a scorer stream.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct S4OracleTokenBpc {
    /// Zero-based token index in `gutenberg_val`.
    pub token: u64,
    /// Target charset-v1 id at this position.
    pub target_token_id: u8,
    /// Per-token loss in bits.
    pub bpc: f64,
}

/// Build `s4_oracle_agreement.v1` from precomputed per-token bpc streams.
pub fn s4_oracle_agreement_from_streams(
    inputs: S4OracleAgreementStreamInputs,
) -> Result<S4OracleAgreementReport, S4OracleAgreementError> {
    validate_mandatory_seed(inputs.seed)?;
    validate_nonzero_hash(
        "tinystories_manifest_self_hash",
        inputs.tinystories_manifest_self_hash,
    )?;
    validate_nonzero_hash(
        "gutenberg_manifest_self_hash",
        inputs.gutenberg_manifest_self_hash,
    )?;
    validate_nonzero_hash("checkpoint_self_hash", inputs.checkpoint_self_hash)?;
    validate_nonzero_hash("corpus_val_sha", inputs.corpus_val_sha)?;
    validate_nonzero_hash(
        "workload_manifest_self_hash",
        inputs.workload_manifest_self_hash,
    )?;
    validate_nonzero_hash("fixture_set_self_hash", inputs.fixture_set_self_hash)?;
    let tolerances = S4InheritedS3OracleTolerances::s3_pinned()?;
    let s3_tolerance_self_hash = tolerances.compute_self_hash()?;
    let per_token = align_streams(&inputs.live, &inputs.denotational, &inputs.artifact)?;
    let summary = summarize_records(&per_token, &tolerances)?;

    let mut report = S4OracleAgreementReport {
        schema: S4_ORACLE_AGREEMENT_SCHEMA.to_owned(),
        tinystories_manifest_self_hash: inputs.tinystories_manifest_self_hash,
        gutenberg_manifest_self_hash: inputs.gutenberg_manifest_self_hash,
        seed: inputs.seed,
        checkpoint_self_hash: inputs.checkpoint_self_hash,
        corpus_val_sha: inputs.corpus_val_sha,
        workload_manifest_self_hash: inputs.workload_manifest_self_hash,
        fixture_set_self_hash: inputs.fixture_set_self_hash,
        bpc_live: summary.bpc_live,
        bpc_denotational: summary.bpc_denotational,
        bpc_artifact: summary.bpc_artifact,
        gap_live_vs_denotational: summary.gap_live_vs_denotational,
        gap_live_vs_artifact: summary.gap_live_vs_artifact,
        gap_denotational_vs_artifact: summary.gap_denotational_vs_artifact,
        per_token,
        s3_tolerance_self_hash,
        outcome: summary.outcome,
        oracle_agreement_self_hash: Hash256::ZERO,
    };
    report.validate_structure()?;
    report.oracle_agreement_self_hash = report.compute_self_hash()?;
    Ok(report)
}

#[derive(Debug, Clone, PartialEq)]
struct S4OracleAgreementSummary {
    bpc_live: f64,
    bpc_denotational: f64,
    bpc_artifact: f64,
    gap_live_vs_denotational: f64,
    gap_live_vs_artifact: f64,
    gap_denotational_vs_artifact: f64,
    outcome: S4OracleAgreementOutcome,
}

fn validate_binding<L, R, A>(
    inputs: &S4OracleAgreementInputs<'_, L, R, A>,
) -> Result<(), S4OracleAgreementError>
where
    L: Evaluator,
    R: Evaluator,
    A: Evaluator,
{
    validate_mandatory_seed(inputs.seed)?;
    if inputs.expected_workload_manifest_self_hash != inputs.workload_manifest_self_hash {
        return Err(S4OracleAgreementError::WorkloadManifestMismatch {
            expected: inputs.expected_workload_manifest_self_hash,
            observed: inputs.workload_manifest_self_hash,
        });
    }
    validate_hash(
        "corpus_val_sha",
        inputs.corpus_val_sha,
        sha256(inputs.gutenberg_val.as_slice()),
    )?;
    validate_nonzero_hash(
        "workload_manifest_self_hash",
        inputs.workload_manifest_self_hash,
    )?;
    Ok(())
}

fn score_per_token_stream<E>(
    scorer_label: &'static str,
    mut evaluator: E,
    val: &TextCharSeq,
) -> Result<Vec<S4OracleTokenBpc>, S4OracleAgreementError>
where
    E: Evaluator,
{
    if val.is_empty() {
        return Err(S4OracleAgreementError::EmptyValidation);
    }

    let mut records = Vec::with_capacity(val.len());
    let mut token_index = 0_u64;
    for chunk in val.as_slice().chunks(S3_SCORE_CHUNK_SIZE) {
        evaluator.reset_state();
        for (offset, &target) in chunk.iter().enumerate() {
            let prefix = &chunk[..offset];
            let target_ix = usize::from(target);
            let output = evaluator.forward(prefix, target_ix);
            let bpc = per_token_bpc(&output, target_ix)?;
            tracing::trace!(
                target: S4_ORACLE_LOG_TARGET,
                event_name = S4_ORACLE_SCORE_EVENT_NAME,
                scorer = scorer_label,
                token = token_index,
                target_token_id = target as u64,
                bpc,
            );
            records.push(S4OracleTokenBpc {
                token: token_index,
                target_token_id: target,
                bpc,
            });
            token_index += 1;
        }
    }
    Ok(records)
}

fn per_token_bpc(
    output: &EvaluatorOutput,
    target_ix: usize,
) -> Result<f64, S4OracleAgreementError> {
    if target_ix >= VOCAB_SIZE {
        return Err(S4OracleAgreementError::TargetOutOfRange { target_ix });
    }
    if output.logits.len() != VOCAB_SIZE {
        return Err(S4OracleAgreementError::LogitsWrongLength {
            len: output.logits.len(),
            expected: VOCAB_SIZE,
        });
    }
    for (index, &value) in output.logits.iter().enumerate() {
        if !value.is_finite() {
            return Err(S4OracleAgreementError::NonFiniteLogit { index, value });
        }
    }
    if !output.target_logprob.is_finite() || output.target_logprob > 0.0 {
        return Err(S4OracleAgreementError::InvalidTargetLogprob {
            value: output.target_logprob,
        });
    }
    let bpc = -output.target_logprob * LOG2_E;
    validate_finite_nonnegative("per_token.bpc", bpc)?;
    Ok(bpc)
}

fn align_streams(
    live: &[S4OracleTokenBpc],
    denotational: &[S4OracleTokenBpc],
    artifact: &[S4OracleTokenBpc],
) -> Result<Vec<S4OracleAgreementTokenRecord>, S4OracleAgreementError> {
    if live.is_empty() {
        return Err(S4OracleAgreementError::EmptyValidation);
    }
    if live.len() != denotational.len() || live.len() != artifact.len() {
        return Err(S4OracleAgreementError::TokenStreamLengthMismatch {
            live: live.len(),
            denotational: denotational.len(),
            artifact: artifact.len(),
        });
    }
    live.iter()
        .zip(denotational)
        .zip(artifact)
        .map(|((live, denotational), artifact)| {
            if live.token != denotational.token
                || live.token != artifact.token
                || live.target_token_id != denotational.target_token_id
                || live.target_token_id != artifact.target_token_id
            {
                return Err(S4OracleAgreementError::TokenStreamIdentityMismatch {
                    token: live.token,
                });
            }
            validate_finite_nonnegative("live.bpc", live.bpc)?;
            validate_finite_nonnegative("denotational.bpc", denotational.bpc)?;
            validate_finite_nonnegative("artifact.bpc", artifact.bpc)?;
            Ok(S4OracleAgreementTokenRecord {
                token: live.token,
                target_token_id: live.target_token_id,
                bpc_live: live.bpc,
                bpc_denotational: denotational.bpc,
                bpc_artifact: artifact.bpc,
                gap_live_vs_denotational: (live.bpc - denotational.bpc).abs(),
                gap_live_vs_artifact: (live.bpc - artifact.bpc).abs(),
                gap_denotational_vs_artifact: (denotational.bpc - artifact.bpc).abs(),
            })
        })
        .collect()
}

fn summarize_records(
    records: &[S4OracleAgreementTokenRecord],
    tolerances: &S4InheritedS3OracleTolerances,
) -> Result<S4OracleAgreementSummary, S4OracleAgreementError> {
    if records.is_empty() {
        return Err(S4OracleAgreementError::EmptyValidation);
    }
    let mut live_sum = 0.0_f64;
    let mut denotational_sum = 0.0_f64;
    let mut artifact_sum = 0.0_f64;
    let mut max_live_denotational = 0.0_f64;
    let mut max_live_artifact = 0.0_f64;
    let mut max_denotational_artifact = 0.0_f64;
    let mut first_failure = None;

    for record in records {
        validate_finite_nonnegative("per_token.bpc_live", record.bpc_live)?;
        validate_finite_nonnegative("per_token.bpc_denotational", record.bpc_denotational)?;
        validate_finite_nonnegative("per_token.bpc_artifact", record.bpc_artifact)?;
        validate_finite_nonnegative(
            "per_token.gap_live_vs_denotational",
            record.gap_live_vs_denotational,
        )?;
        validate_finite_nonnegative(
            "per_token.gap_live_vs_artifact",
            record.gap_live_vs_artifact,
        )?;
        validate_finite_nonnegative(
            "per_token.gap_denotational_vs_artifact",
            record.gap_denotational_vs_artifact,
        )?;

        live_sum += record.bpc_live;
        denotational_sum += record.bpc_denotational;
        artifact_sum += record.bpc_artifact;
        max_live_denotational = max_live_denotational.max(record.gap_live_vs_denotational);
        max_live_artifact = max_live_artifact.max(record.gap_live_vs_artifact);
        max_denotational_artifact =
            max_denotational_artifact.max(record.gap_denotational_vs_artifact);

        let token_max = record
            .gap_live_vs_denotational
            .max(record.gap_live_vs_artifact)
            .max(record.gap_denotational_vs_artifact);
        let token_failed = record.gap_live_vs_denotational > tolerances.live_vs_denotational_bpc
            || record.gap_live_vs_artifact > tolerances.live_vs_artifact_bpc
            || record.gap_denotational_vs_artifact > tolerances.denotational_vs_artifact_bpc;
        if token_failed && first_failure.is_none() {
            first_failure = Some((record.token, token_max));
        }
    }

    let count = records.len() as f64;
    let outcome = match first_failure {
        None => S4OracleAgreementOutcome::Agree,
        Some((failing_token, max_gap)) => S4OracleAgreementOutcome::Disagree {
            failing_token,
            max_gap,
        },
    };
    Ok(S4OracleAgreementSummary {
        bpc_live: live_sum / count,
        bpc_denotational: denotational_sum / count,
        bpc_artifact: artifact_sum / count,
        gap_live_vs_denotational: max_live_denotational,
        gap_live_vs_artifact: max_live_artifact,
        gap_denotational_vs_artifact: max_denotational_artifact,
        outcome,
    })
}

fn validate_hash(
    field: &'static str,
    expected: Hash256,
    observed: Hash256,
) -> Result<(), S4OracleAgreementError> {
    if expected == observed {
        Ok(())
    } else {
        Err(S4OracleAgreementError::HashMismatch {
            field,
            expected,
            observed,
        })
    }
}

fn validate_nonzero_hash(field: &'static str, hash: Hash256) -> Result<(), S4OracleAgreementError> {
    if hash == Hash256::ZERO {
        Err(S4OracleAgreementError::MissingHash { field })
    } else {
        Ok(())
    }
}

fn validate_mandatory_seed(seed: u64) -> Result<(), S4OracleAgreementError> {
    validate_s4_seed(seed)?;
    if seed != S4_ORACLE_MANDATORY_SEED {
        return Err(S4OracleAgreementError::NonMandatorySeed { seed });
    }
    Ok(())
}

fn validate_finite_nonnegative(
    field: &'static str,
    value: f64,
) -> Result<(), S4OracleAgreementError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(S4OracleAgreementError::NonFiniteOrNegative { field, value })
    }
}

const fn same_f64_bits(left: f64, right: f64) -> bool {
    left.to_bits() == right.to_bits()
}

/// Errors from S4 oracle agreement.
#[derive(Debug)]
pub enum S4OracleAgreementError {
    /// Validation text must not be empty.
    EmptyValidation,
    /// A required lineage hash was zero.
    MissingHash {
        /// Rejected field.
        field: &'static str,
    },
    /// A supplied hash did not match observed bytes.
    HashMismatch {
        /// Rejected field.
        field: &'static str,
        /// Expected hash.
        expected: Hash256,
        /// Observed hash.
        observed: Hash256,
    },
    /// Expected and observed workload manifests did not match.
    WorkloadManifestMismatch {
        /// Expected workload manifest self-hash.
        expected: Hash256,
        /// Observed workload manifest self-hash.
        observed: Hash256,
    },
    /// D15 mandatory report is seed 0 only.
    NonMandatorySeed {
        /// Observed seed.
        seed: u64,
    },
    /// S3 tolerance payload is missing an inherited gate.
    MissingS3Tolerance {
        /// Missing field.
        field: &'static str,
    },
    /// The tolerance payload schema was not pinned.
    InvalidToleranceSchema {
        /// Observed schema.
        observed: String,
    },
    /// Report schema did not match `s4_oracle_agreement.v1`.
    InvalidSchema {
        /// Observed schema.
        observed: String,
    },
    /// A floating-point field must be finite and non-negative.
    NonFiniteOrNegative {
        /// Rejected field.
        field: &'static str,
        /// Rejected value.
        value: f64,
    },
    /// Scorer logits had the wrong vocabulary length.
    LogitsWrongLength {
        /// Observed length.
        len: usize,
        /// Expected vocabulary size.
        expected: usize,
    },
    /// Scorer emitted a non-finite logit.
    NonFiniteLogit {
        /// Logit index.
        index: usize,
        /// Observed value.
        value: f32,
    },
    /// Target id was outside the S3/S4 text vocabulary.
    TargetOutOfRange {
        /// Observed target index.
        target_ix: usize,
    },
    /// Evaluator target log-probability was invalid.
    InvalidTargetLogprob {
        /// Observed value.
        value: f64,
    },
    /// The three scorer streams had different lengths.
    TokenStreamLengthMismatch {
        /// Live stream length.
        live: usize,
        /// Denotational stream length.
        denotational: usize,
        /// Artifact stream length.
        artifact: usize,
    },
    /// Per-token stream identities drifted.
    TokenStreamIdentityMismatch {
        /// First live token index where identity mismatch was observed.
        token: u64,
    },
    /// Stored summary fields do not match per-token rows and S3 tolerances.
    SummaryMismatch,
    /// Tolerance self-hash did not recompute from S3.
    ToleranceHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// Report self-hash mismatch.
    SelfHashMismatch {
        /// Recomputed self-hash.
        expected: Hash256,
        /// Stored self-hash.
        observed: Hash256,
    },
    /// S3 score primitive failed.
    Score(ScoreError),
    /// S4 schema validation failed.
    Schema(S4SchemaError),
    /// Canonical JSON serialization or hashing failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for S4OracleAgreementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyValidation => {
                f.write_str("S4 oracle agreement requires non-empty validation text")
            }
            Self::MissingHash { field } => {
                write!(f, "S4 oracle agreement field {field} must be non-zero")
            }
            Self::HashMismatch {
                field,
                expected,
                observed,
            } => write!(
                f,
                "S4 oracle agreement hash mismatch for {field}: expected {expected}, observed {observed}"
            ),
            Self::WorkloadManifestMismatch { expected, observed } => write!(
                f,
                "S4 oracle agreement workload manifest mismatch: expected {expected}, observed {observed}"
            ),
            Self::NonMandatorySeed { seed } => write!(
                f,
                "S4 D15 mandatory oracle agreement report is seed {S4_ORACLE_MANDATORY_SEED}, got {seed}"
            ),
            Self::MissingS3Tolerance { field } => {
                write!(f, "S3 pinned oracle tolerance {field} is missing")
            }
            Self::InvalidToleranceSchema { observed } => write!(
                f,
                "S4 inherited tolerance schema must be {S4_S3_ORACLE_TOLERANCE_SCHEMA}, got {observed}"
            ),
            Self::InvalidSchema { observed } => write!(
                f,
                "S4 oracle agreement schema must be {S4_ORACLE_AGREEMENT_SCHEMA}, got {observed}"
            ),
            Self::NonFiniteOrNegative { field, value } => {
                write!(
                    f,
                    "S4 oracle agreement field {field} must be finite and non-negative, got {value}"
                )
            }
            Self::LogitsWrongLength { len, expected } => {
                write!(
                    f,
                    "S4 oracle scorer logits length {len} does not match vocab size {expected}"
                )
            }
            Self::NonFiniteLogit { index, value } => {
                write!(f, "S4 oracle scorer logit {index} is non-finite: {value}")
            }
            Self::TargetOutOfRange { target_ix } => {
                write!(
                    f,
                    "S4 oracle target index {target_ix} is outside vocab size {VOCAB_SIZE}"
                )
            }
            Self::InvalidTargetLogprob { value } => {
                write!(
                    f,
                    "S4 oracle scorer target log-probability is invalid: {value}"
                )
            }
            Self::TokenStreamLengthMismatch {
                live,
                denotational,
                artifact,
            } => write!(
                f,
                "S4 oracle scorer stream length mismatch: live {live}, denotational {denotational}, artifact {artifact}"
            ),
            Self::TokenStreamIdentityMismatch { token } => write!(
                f,
                "S4 oracle scorer stream token identity mismatch at token {token}"
            ),
            Self::SummaryMismatch => {
                f.write_str("S4 oracle agreement summary does not match per-token rows")
            }
            Self::ToleranceHashMismatch { expected, observed } => write!(
                f,
                "S4 oracle S3 tolerance hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::SelfHashMismatch { expected, observed } => write!(
                f,
                "S4 oracle agreement self-hash mismatch: expected {expected}, observed {observed}"
            ),
            Self::Score(error) => write!(f, "{error}"),
            Self::Schema(error) => write!(f, "{error}"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
        }
    }
}

impl Error for S4OracleAgreementError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Score(error) => Some(error),
            Self::Schema(error) => Some(error),
            Self::CanonicalJson(error) => Some(error),
            Self::EmptyValidation
            | Self::MissingHash { .. }
            | Self::HashMismatch { .. }
            | Self::WorkloadManifestMismatch { .. }
            | Self::NonMandatorySeed { .. }
            | Self::MissingS3Tolerance { .. }
            | Self::InvalidToleranceSchema { .. }
            | Self::InvalidSchema { .. }
            | Self::NonFiniteOrNegative { .. }
            | Self::LogitsWrongLength { .. }
            | Self::NonFiniteLogit { .. }
            | Self::TargetOutOfRange { .. }
            | Self::InvalidTargetLogprob { .. }
            | Self::TokenStreamLengthMismatch { .. }
            | Self::TokenStreamIdentityMismatch { .. }
            | Self::SummaryMismatch
            | Self::ToleranceHashMismatch { .. }
            | Self::SelfHashMismatch { .. } => None,
        }
    }
}

impl From<ScoreError> for S4OracleAgreementError {
    fn from(error: ScoreError) -> Self {
        Self::Score(error)
    }
}

impl From<S4SchemaError> for S4OracleAgreementError {
    fn from(error: S4SchemaError) -> Self {
        Self::Schema(error)
    }
}

impl From<CanonicalJsonError> for S4OracleAgreementError {
    fn from(error: CanonicalJsonError) -> Self {
        Self::CanonicalJson(error)
    }
}
