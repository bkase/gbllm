//! Phase-specific S3 live-vs-oracle surface agreement.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;

use gbf_artifact::{AggregationKind, VOCAB_SIZE};
use gbf_foundation::{
    CanonicalJson, CanonicalJsonError, DomainHash, Hash256, self_hash_omitting_fields,
};
use gbf_workload::PromptId;
use serde::{Deserialize, Serialize};

use crate::artifact::ArtifactOracleProduct;
use crate::denotational::{
    DenotationalOracleProduct, Observation, S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD,
    SemanticCheckpoint,
};

const PRODUCT_SCHEMA_VERSION: &str = "1";

/// Schema id for the S3 oracle agreement product.
pub const S3_ORACLE_AGREEMENT_SCHEMA: &str = "s3_oracle_agreement.v1";

/// Stable fallback tag for S3 denotational fallback use.
pub const S3_DENOTATIONAL_FALLBACK_TAG: &str = "S3DenotationalFallback";
/// Stable fallback tag for S3 artifact fallback use.
pub const S3_ARTIFACT_FALLBACK_TAG: &str = "S3ArtifactFallback";
/// Stable fallback tag for oracle-derived fixture live observations.
pub const S3_LIVE_OBSERVATION_FIXTURE_TAG: &str = "S3LiveObservationFixture";
/// Bead that owns the real S3 live-observation capture path.
pub const S3_LIVE_OBSERVATION_REAL_OWNER_BEAD: &str = "bd-1ybu";

/// Typed fallback/provenance tags exposed by `s3_oracle_agreement.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[allow(clippy::enum_variant_names)]
pub enum OracleFallbackTag {
    /// The denotational oracle used the S3 fallback evaluator.
    S3DenotationalFallback,
    /// The artifact oracle used the S3 fallback evaluator.
    S3ArtifactFallback,
    /// Live observations were fixture-derived from oracle observations.
    S3LiveObservationFixture,
}

impl OracleFallbackTag {
    /// Stable serialized/logging label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3DenotationalFallback => S3_DENOTATIONAL_FALLBACK_TAG,
            Self::S3ArtifactFallback => S3_ARTIFACT_FALLBACK_TAG,
            Self::S3LiveObservationFixture => S3_LIVE_OBSERVATION_FIXTURE_TAG,
        }
    }
}

/// Provenance class for the training observations compared against oracles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiveObservationSourceKind {
    /// Captured from the live training runs.
    RealTrainCapture,
    /// Fixture-only substitution derived from oracle outputs.
    OracleDerivedFixture,
}

impl LiveObservationSourceKind {
    /// Stable logging label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RealTrainCapture => "real_train_capture",
            Self::OracleDerivedFixture => "oracle_derived_fixture",
        }
    }
}

/// Source metadata for live training observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiveObservationSource {
    /// Source class for the supplied training observations.
    pub kind: LiveObservationSourceKind,
    /// Owner bead for replacing a fixture source with real live capture.
    pub real_owner_bead: Option<String>,
}

impl LiveObservationSource {
    /// Real live training capture source.
    #[must_use]
    pub fn real_train_capture() -> Self {
        Self {
            kind: LiveObservationSourceKind::RealTrainCapture,
            real_owner_bead: None,
        }
    }

    /// Fixture source derived from oracle observations.
    #[must_use]
    pub fn oracle_derived_fixture(real_owner_bead: impl Into<String>) -> Self {
        Self {
            kind: LiveObservationSourceKind::OracleDerivedFixture,
            real_owner_bead: Some(real_owner_bead.into()),
        }
    }

    fn validate(&self) -> Result<(), AgreementError> {
        match self.kind {
            LiveObservationSourceKind::RealTrainCapture => {
                if self.real_owner_bead.is_some() {
                    return Err(AgreementError::InvalidLiveObservationSource {
                        reason: "real train capture must not name a fallback owner",
                    });
                }
            }
            LiveObservationSourceKind::OracleDerivedFixture => {
                if self.real_owner_bead.as_deref().is_none_or(str::is_empty) {
                    return Err(AgreementError::InvalidLiveObservationSource {
                        reason: "oracle-derived fixture live observations must name the real-source owner bead",
                    });
                }
            }
        }
        Ok(())
    }

    fn fallback_tag(&self) -> Option<OracleFallbackTag> {
        match self.kind {
            LiveObservationSourceKind::RealTrainCapture => None,
            LiveObservationSourceKind::OracleDerivedFixture => {
                Some(OracleFallbackTag::S3LiveObservationFixture)
            }
        }
    }
}

/// S3 training phase whose live surface is being compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhaseId {
    /// Phase A teacher live surface compared against the reference bundle.
    PhaseA,
    /// Phase D student live surface compared against the model artifact.
    PhaseD,
}

impl PhaseId {
    /// Stable logging label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PhaseA => "phase_a",
            Self::PhaseD => "phase_d",
        }
    }
}

/// Agreement comparator policy for one phase.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementPolicy {
    /// Phase to compare.
    pub phase: PhaseId,
    /// Maximum tolerated per-token/per-vocab-row logit absolute difference.
    pub max_logit_abs_diff: f32,
    /// Whether argmax token equality is required.
    pub require_argmax_token_match: bool,
    /// Aggregation provenance; only per-token/per-vocab-row is valid here.
    pub aggregation: AggregationKind,
}

impl AgreementPolicy {
    /// Construct a Phase A live-teacher-vs-bundle policy.
    #[must_use]
    pub const fn phase_a(max_logit_abs_diff: f32, require_argmax_token_match: bool) -> Self {
        Self {
            phase: PhaseId::PhaseA,
            max_logit_abs_diff,
            require_argmax_token_match,
            aggregation: AggregationKind::PerTokenPerVocabRow,
        }
    }

    /// Construct a Phase D live-student-vs-artifact policy.
    #[must_use]
    pub const fn phase_d(max_logit_abs_diff: f32, require_argmax_token_match: bool) -> Self {
        Self {
            phase: PhaseId::PhaseD,
            max_logit_abs_diff,
            require_argmax_token_match,
            aggregation: AggregationKind::PerTokenPerVocabRow,
        }
    }

    fn validate(self) -> Result<(), AgreementError> {
        if !self.max_logit_abs_diff.is_finite() || self.max_logit_abs_diff < 0.0 {
            return Err(AgreementError::InvalidTolerance {
                observed: self.max_logit_abs_diff,
            });
        }
        if self.aggregation != AggregationKind::PerTokenPerVocabRow {
            return Err(AgreementError::InvalidAggregation {
                observed: self.aggregation,
            });
        }
        Ok(())
    }
}

/// One live training observation in capture order.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainObservation {
    /// S3 seed.
    pub seed: u64,
    /// Training phase.
    pub phase: PhaseId,
    /// Workload prompt id.
    pub prompt_id: PromptId,
    /// Semantic checkpoint.
    pub checkpoint: SemanticCheckpoint,
    /// Generated-step index.
    pub step: u32,
    /// Checkpoint-specific observation.
    pub observation: Observation,
}

/// Live training observations in runner capture order.
#[derive(Debug, Clone, PartialEq)]
pub struct TrainObservations(pub Vec<TrainObservation>);

impl TrainObservations {
    /// Construct an empty live-observation map.
    #[must_use]
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Insert a checkpoint-specific live observation.
    pub fn insert(
        &mut self,
        seed: u64,
        phase: PhaseId,
        prompt_id: PromptId,
        checkpoint: SemanticCheckpoint,
        step: u32,
        observation: Observation,
    ) -> Result<(), AgreementError> {
        if observation.checkpoint() != checkpoint {
            return Err(AgreementError::CheckpointMismatch {
                key: checkpoint,
                observed: observation.checkpoint(),
            });
        }
        if self.0.iter().any(|entry| {
            entry.seed == seed
                && entry.phase == phase
                && entry.prompt_id == prompt_id
                && entry.checkpoint == checkpoint
                && entry.step == step
        }) {
            return Err(AgreementError::DuplicateTrainObservation {
                seed,
                phase,
                prompt_id: prompt_id.to_string(),
                checkpoint,
                step,
            });
        }
        self.0.push(TrainObservation {
            seed,
            phase,
            prompt_id,
            checkpoint,
            step,
            observation,
        });
        Ok(())
    }

    /// Iterate observations in runner capture order.
    pub fn iter(&self) -> impl Iterator<Item = &TrainObservation> {
        self.0.iter()
    }

    /// Number of captured live observations.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether no live observations were captured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Default for TrainObservations {
    fn default() -> Self {
        Self::new()
    }
}

/// Training observations plus explicit provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveTrainCapture {
    /// Live observations consumed by the agreement comparator.
    pub observations: TrainObservations,
    /// Source metadata for those observations.
    pub source: LiveObservationSource,
}

impl LiveTrainCapture {
    /// Construct a live-capture bundle with explicit provenance.
    pub fn new(
        observations: TrainObservations,
        source: LiveObservationSource,
    ) -> Result<Self, AgreementError> {
        source.validate()?;
        Ok(Self {
            observations,
            source,
        })
    }

    /// Construct a real live training capture bundle.
    pub fn real_train_capture(observations: TrainObservations) -> Result<Self, AgreementError> {
        Self::new(observations, LiveObservationSource::real_train_capture())
    }

    /// Construct a fixture-only bundle derived from oracle observations.
    pub fn oracle_derived_fixture(
        observations: TrainObservations,
        real_owner_bead: impl Into<String>,
    ) -> Result<Self, AgreementError> {
        Self::new(
            observations,
            LiveObservationSource::oracle_derived_fixture(real_owner_bead),
        )
    }
}

/// One canonical S3 agreement record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementRecord {
    /// S3 seed.
    pub seed: u64,
    /// Workload prompt id.
    pub prompt_id: String,
    /// Semantic checkpoint under comparison.
    pub checkpoint: SemanticCheckpoint,
    /// Forced generated-step index.
    pub step: u32,
    /// Phase that gates this record.
    pub phase: PhaseId,
    /// Aggregation provenance.
    pub aggregation_kind: AggregationKind,
    /// Phase A live-teacher-vs-bundle max abs diff, absent in Phase D.
    pub train_vs_bundle_max_abs_diff: Option<f32>,
    /// Phase A live-teacher-vs-bundle argmax equality, absent in Phase D.
    pub train_vs_bundle_argmax_match: Option<bool>,
    /// Phase A live-teacher-vs-bundle per-token KL for aligned logits rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub train_vs_bundle_per_token_kl: Option<f32>,
    /// Phase A gate result, absent in Phase D.
    pub train_vs_bundle_pass: Option<bool>,
    /// Phase D live-student-vs-artifact max abs diff, absent in Phase A.
    pub train_vs_artifact_max_abs_diff: Option<f32>,
    /// Phase D live-student-vs-artifact argmax equality, absent in Phase A.
    pub train_vs_artifact_argmax_match: Option<bool>,
    /// Phase D live-student-vs-artifact per-token KL for aligned logits rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub train_vs_artifact_per_token_kl: Option<f32>,
    /// Phase D gate result, absent in Phase A.
    pub train_vs_artifact_pass: Option<bool>,
    /// Bundle-vs-artifact report-only max abs diff.
    pub bundle_vs_artifact_max_abs_diff: Option<f32>,
    /// Bundle-vs-artifact report-only argmax equality.
    pub bundle_vs_artifact_argmax_match: Option<bool>,
    /// Bundle-vs-artifact report-only per-token KL for aligned logits rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_vs_artifact_per_token_kl: Option<f32>,
}

impl AgreementRecord {
    fn validate(&self) -> Result<(), AgreementError> {
        if self.aggregation_kind != AggregationKind::PerTokenPerVocabRow {
            return Err(AgreementError::InvalidAggregation {
                observed: self.aggregation_kind,
            });
        }
        for (name, value) in [
            (
                "train_vs_bundle_max_abs_diff",
                self.train_vs_bundle_max_abs_diff,
            ),
            (
                "train_vs_artifact_max_abs_diff",
                self.train_vs_artifact_max_abs_diff,
            ),
            (
                "bundle_vs_artifact_max_abs_diff",
                self.bundle_vs_artifact_max_abs_diff,
            ),
            (
                "train_vs_bundle_per_token_kl",
                self.train_vs_bundle_per_token_kl,
            ),
            (
                "train_vs_artifact_per_token_kl",
                self.train_vs_artifact_per_token_kl,
            ),
            (
                "bundle_vs_artifact_per_token_kl",
                self.bundle_vs_artifact_per_token_kl,
            ),
        ] {
            if let Some(value) = value
                && (!value.is_finite() || value < 0.0)
            {
                return Err(AgreementError::InvalidRecordDiff { name, value });
            }
        }

        match self.phase {
            PhaseId::PhaseA => {
                if self.train_vs_bundle_max_abs_diff.is_none()
                    || self.train_vs_bundle_argmax_match.is_none()
                    || self.train_vs_bundle_pass.is_none()
                    || self.train_vs_artifact_max_abs_diff.is_some()
                    || self.train_vs_artifact_argmax_match.is_some()
                    || self.train_vs_artifact_per_token_kl.is_some()
                    || self.train_vs_artifact_pass.is_some()
                {
                    return Err(AgreementError::OptionalFieldDiscipline { phase: self.phase });
                }
            }
            PhaseId::PhaseD => {
                if self.train_vs_artifact_max_abs_diff.is_none()
                    || self.train_vs_artifact_argmax_match.is_none()
                    || self.train_vs_artifact_pass.is_none()
                    || self.train_vs_bundle_max_abs_diff.is_some()
                    || self.train_vs_bundle_argmax_match.is_some()
                    || self.train_vs_bundle_per_token_kl.is_some()
                    || self.train_vs_bundle_pass.is_some()
                {
                    return Err(AgreementError::OptionalFieldDiscipline { phase: self.phase });
                }
            }
        }
        Ok(())
    }
}

/// Marker for Phase A record builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseARecord;

/// Marker for Phase D record builders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhaseDRecord;

/// Phase-aware agreement-record builder.
#[derive(Debug, Clone)]
pub struct AgreementRecordBuilder<P> {
    record: AgreementRecord,
    _phase: PhantomData<P>,
}

impl AgreementRecordBuilder<PhaseARecord> {
    /// Start a Phase A record with default seed zero and per-token aggregation.
    #[must_use]
    pub fn for_phase_a(
        prompt_id: impl Into<String>,
        checkpoint: SemanticCheckpoint,
        step: u32,
    ) -> Self {
        Self::for_phase_a_with_seed(
            0,
            prompt_id,
            checkpoint,
            step,
            AggregationKind::PerTokenPerVocabRow,
        )
    }

    /// Start a Phase A record.
    #[must_use]
    pub fn for_phase_a_with_seed(
        seed: u64,
        prompt_id: impl Into<String>,
        checkpoint: SemanticCheckpoint,
        step: u32,
        aggregation_kind: AggregationKind,
    ) -> Self {
        Self {
            record: empty_record(
                seed,
                prompt_id,
                checkpoint,
                step,
                PhaseId::PhaseA,
                aggregation_kind,
            ),
            _phase: PhantomData,
        }
    }

    /// Populate the Phase A live-teacher-vs-bundle gated comparison.
    #[must_use]
    pub fn with_train_vs_bundle(
        mut self,
        max_abs_diff: f32,
        argmax_match: bool,
        passed: bool,
    ) -> Self {
        self.record.train_vs_bundle_max_abs_diff = Some(max_abs_diff);
        self.record.train_vs_bundle_argmax_match = Some(argmax_match);
        self.record.train_vs_bundle_pass = Some(passed);
        self
    }

    /// Populate Phase A per-token KL when aligned logits rows define it.
    #[must_use]
    pub fn with_train_vs_bundle_per_token_kl(mut self, per_token_kl: Option<f32>) -> Self {
        self.record.train_vs_bundle_per_token_kl = per_token_kl;
        self
    }
}

impl AgreementRecordBuilder<PhaseDRecord> {
    /// Start a Phase D record with default seed zero and per-token aggregation.
    #[must_use]
    pub fn for_phase_d(
        prompt_id: impl Into<String>,
        checkpoint: SemanticCheckpoint,
        step: u32,
    ) -> Self {
        Self::for_phase_d_with_seed(
            0,
            prompt_id,
            checkpoint,
            step,
            AggregationKind::PerTokenPerVocabRow,
        )
    }

    /// Start a Phase D record.
    #[must_use]
    pub fn for_phase_d_with_seed(
        seed: u64,
        prompt_id: impl Into<String>,
        checkpoint: SemanticCheckpoint,
        step: u32,
        aggregation_kind: AggregationKind,
    ) -> Self {
        Self {
            record: empty_record(
                seed,
                prompt_id,
                checkpoint,
                step,
                PhaseId::PhaseD,
                aggregation_kind,
            ),
            _phase: PhantomData,
        }
    }

    /// Populate the Phase D live-student-vs-artifact gated comparison.
    #[must_use]
    pub fn with_train_vs_artifact(
        mut self,
        max_abs_diff: f32,
        argmax_match: bool,
        passed: bool,
    ) -> Self {
        self.record.train_vs_artifact_max_abs_diff = Some(max_abs_diff);
        self.record.train_vs_artifact_argmax_match = Some(argmax_match);
        self.record.train_vs_artifact_pass = Some(passed);
        self
    }

    /// Populate Phase D per-token KL when aligned logits rows define it.
    #[must_use]
    pub fn with_train_vs_artifact_per_token_kl(mut self, per_token_kl: Option<f32>) -> Self {
        self.record.train_vs_artifact_per_token_kl = per_token_kl;
        self
    }
}

impl<P> AgreementRecordBuilder<P> {
    /// Populate the report-only bundle-vs-artifact comparison.
    #[must_use]
    pub fn with_bundle_vs_artifact(mut self, max_abs_diff: f32, argmax_match: bool) -> Self {
        self.record.bundle_vs_artifact_max_abs_diff = Some(max_abs_diff);
        self.record.bundle_vs_artifact_argmax_match = Some(argmax_match);
        self
    }

    /// Populate bundle-vs-artifact per-token KL when aligned logits rows define it.
    #[must_use]
    pub fn with_bundle_vs_artifact_per_token_kl(mut self, per_token_kl: Option<f32>) -> Self {
        self.record.bundle_vs_artifact_per_token_kl = per_token_kl;
        self
    }

    /// Finish a validated agreement record.
    pub fn build(self) -> Result<AgreementRecord, AgreementError> {
        self.record.validate()?;
        Ok(self.record)
    }
}

/// Canonical `s3_oracle_agreement.v1` product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgreementProduct {
    /// Pinned schema literal.
    pub schema: String,
    /// Agreement records in runner capture order.
    pub records: Vec<AgreementRecord>,
    /// Whether all Phase A gated records passed.
    pub phase_a_pass: bool,
    /// Whether all Phase D gated records passed.
    pub phase_d_pass: bool,
    /// Whether all phase-specific gates passed.
    pub overall_pass: bool,
    /// Source metadata for training observations compared against oracle output.
    pub live_observation_source: LiveObservationSource,
    /// Named fallback/provenance paths used while producing this product.
    pub fallback_used: Vec<OracleFallbackTag>,
    /// Canonical self-hash with this field omitted.
    pub agreement_self_hash: Hash256,
}

impl AgreementProduct {
    /// Construct and canonicalize an agreement product.
    pub fn new(
        records: Vec<AgreementRecord>,
        fallback_used: Vec<OracleFallbackTag>,
        live_observation_source: LiveObservationSource,
    ) -> Result<Self, AgreementError> {
        if records.is_empty() {
            return Err(AgreementError::EmptyRecords);
        }
        live_observation_source.validate()?;
        for record in &records {
            record.validate()?;
        }

        let mut seen = BTreeSet::new();
        for record in &records {
            let key = (
                record.seed,
                record.phase,
                record.prompt_id.clone(),
                record.checkpoint,
                record.step,
            );
            if !seen.insert(key) {
                return Err(AgreementError::DuplicateRecord {
                    seed: record.seed,
                    phase: record.phase,
                    prompt_id: record.prompt_id.clone(),
                    checkpoint: record.checkpoint,
                    step: record.step,
                });
            }
        }

        let mut fallback_used = fallback_used;
        if let Some(tag) = live_observation_source.fallback_tag() {
            fallback_used.push(tag);
        }
        fallback_used.sort_by_key(|tag| tag.as_str());
        fallback_used.dedup();

        let phase_a_pass = records
            .iter()
            .filter(|record| record.phase == PhaseId::PhaseA)
            .all(|record| record.train_vs_bundle_pass == Some(true));
        let phase_d_pass = records
            .iter()
            .filter(|record| record.phase == PhaseId::PhaseD)
            .all(|record| record.train_vs_artifact_pass == Some(true));
        let overall_pass = phase_a_pass && phase_d_pass;

        let mut product = Self {
            schema: S3_ORACLE_AGREEMENT_SCHEMA.to_owned(),
            records,
            phase_a_pass,
            phase_d_pass,
            overall_pass,
            live_observation_source,
            fallback_used,
            agreement_self_hash: Hash256::ZERO,
        };
        product.agreement_self_hash = product.compute_self_hash()?;
        Ok(product)
    }

    /// DomainHash context for agreement products.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-oracle",
            "AgreementProduct",
            S3_ORACLE_AGREEMENT_SCHEMA,
            PRODUCT_SCHEMA_VERSION,
        )
    }

    /// Compute the canonical self-hash.
    pub fn compute_self_hash(&self) -> Result<Hash256, AgreementError> {
        self_hash_omitting_fields(Self::domain(), self, "agreement_self_hash", &[])
            .map_err(AgreementError::CanonicalJson)
    }

    /// Canonical JSON bytes for the product.
    pub fn canonical_json_bytes(&self) -> Result<Vec<u8>, AgreementError> {
        CanonicalJson::to_vec(self).map_err(AgreementError::CanonicalJson)
    }
}

/// Compare one phase. Panics on invalid inputs; use [`try_compare`] when the
/// caller needs a recoverable error.
pub fn compare(
    train_obs: TrainObservations,
    denot: DenotationalOracleProduct,
    artifact: ArtifactOracleProduct,
    policy: AgreementPolicy,
) -> AgreementProduct {
    let fallback_used = fallback_tags(&denot, &artifact);
    try_compare_with_source(
        train_obs,
        &denot,
        &artifact,
        policy,
        fallback_used,
        LiveObservationSource::real_train_capture(),
    )
    .expect("agreement comparison inputs must be valid")
}

/// Compare one phase using real live training observations.
pub fn try_compare(
    train_obs: TrainObservations,
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    policy: AgreementPolicy,
    fallback_used: Vec<OracleFallbackTag>,
) -> Result<AgreementProduct, AgreementError> {
    try_compare_with_source(
        train_obs,
        denot,
        artifact,
        policy,
        fallback_used,
        LiveObservationSource::real_train_capture(),
    )
}

/// Compare one phase and return a recoverable error with explicit source metadata.
pub fn try_compare_with_source(
    train_obs: TrainObservations,
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    policy: AgreementPolicy,
    fallback_used: Vec<OracleFallbackTag>,
    live_observation_source: LiveObservationSource,
) -> Result<AgreementProduct, AgreementError> {
    let records = records_for_policy(&train_obs, denot, artifact, policy)?;
    AgreementProduct::new(records, fallback_used, live_observation_source)
}

/// Compare Phase A and Phase D in one canonical product using real live capture.
pub fn try_compare_phases(
    train_obs: TrainObservations,
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    phase_a_policy: AgreementPolicy,
    phase_d_policy: AgreementPolicy,
    fallback_used: Vec<OracleFallbackTag>,
) -> Result<AgreementProduct, AgreementError> {
    try_compare_phases_with_source(
        train_obs,
        denot,
        artifact,
        phase_a_policy,
        phase_d_policy,
        fallback_used,
        LiveObservationSource::real_train_capture(),
    )
}

/// Compare Phase A and Phase D with explicit live-observation source metadata.
pub fn try_compare_phases_with_source(
    train_obs: TrainObservations,
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    phase_a_policy: AgreementPolicy,
    phase_d_policy: AgreementPolicy,
    fallback_used: Vec<OracleFallbackTag>,
    live_observation_source: LiveObservationSource,
) -> Result<AgreementProduct, AgreementError> {
    if phase_a_policy.phase != PhaseId::PhaseA {
        return Err(AgreementError::PolicyPhaseMismatch {
            expected: PhaseId::PhaseA,
            observed: phase_a_policy.phase,
        });
    }
    if phase_d_policy.phase != PhaseId::PhaseD {
        return Err(AgreementError::PolicyPhaseMismatch {
            expected: PhaseId::PhaseD,
            observed: phase_d_policy.phase,
        });
    }
    let mut records = records_for_policy(&train_obs, denot, artifact, phase_a_policy)?;
    records.extend(records_for_policy(
        &train_obs,
        denot,
        artifact,
        phase_d_policy,
    )?);
    AgreementProduct::new(records, fallback_used, live_observation_source)
}

/// Fallback tags implied by the oracle products.
#[must_use]
pub fn fallback_tags(
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
) -> Vec<OracleFallbackTag> {
    let mut tags = Vec::new();
    if denot.real_owner_bead == Some(S3_DENOTATIONAL_FALLBACK_REAL_OWNER_BEAD) {
        tags.push(OracleFallbackTag::S3DenotationalFallback);
    }
    if artifact.real_owner_bead == Some(crate::artifact::S3_ARTIFACT_FALLBACK_REAL_OWNER_BEAD) {
        tags.push(OracleFallbackTag::S3ArtifactFallback);
    }
    tags
}

fn records_for_policy(
    train_obs: &TrainObservations,
    denot: &DenotationalOracleProduct,
    artifact: &ArtifactOracleProduct,
    policy: AgreementPolicy,
) -> Result<Vec<AgreementRecord>, AgreementError> {
    policy.validate()?;
    let mut records = Vec::new();
    for train in train_obs.iter() {
        if train.phase != policy.phase || !is_agreement_gated_checkpoint(train.checkpoint) {
            continue;
        }
        let bundle = denot
            .observations
            .0
            .get(&(train.prompt_id.clone(), train.checkpoint, train.step))
            .ok_or_else(|| AgreementError::MissingBundleObservation {
                prompt_id: train.prompt_id.to_string(),
                checkpoint: train.checkpoint,
                step: train.step,
            })?;
        let artifact_observation = artifact
            .observations
            .0
            .get(&(train.prompt_id.clone(), train.checkpoint, train.step))
            .ok_or_else(|| AgreementError::MissingArtifactObservation {
                prompt_id: train.prompt_id.to_string(),
                checkpoint: train.checkpoint,
                step: train.step,
            })?;

        let bundle_vs_artifact = surface_comparison(bundle, artifact_observation)?;
        let record = match policy.phase {
            PhaseId::PhaseA => {
                let train_vs_bundle = surface_comparison(&train.observation, bundle)?;
                AgreementRecordBuilder::for_phase_a_with_seed(
                    train.seed,
                    train.prompt_id.to_string(),
                    train.checkpoint,
                    train.step,
                    policy.aggregation,
                )
                .with_train_vs_bundle(
                    train_vs_bundle.max_abs_diff,
                    train_vs_bundle.argmax_match,
                    comparison_passes(train_vs_bundle, policy),
                )
                .with_train_vs_bundle_per_token_kl(train_vs_bundle.per_token_kl)
                .with_bundle_vs_artifact(
                    bundle_vs_artifact.max_abs_diff,
                    bundle_vs_artifact.argmax_match,
                )
                .with_bundle_vs_artifact_per_token_kl(bundle_vs_artifact.per_token_kl)
                .build()?
            }
            PhaseId::PhaseD => {
                let train_vs_artifact =
                    surface_comparison(&train.observation, artifact_observation)?;
                AgreementRecordBuilder::for_phase_d_with_seed(
                    train.seed,
                    train.prompt_id.to_string(),
                    train.checkpoint,
                    train.step,
                    policy.aggregation,
                )
                .with_train_vs_artifact(
                    train_vs_artifact.max_abs_diff,
                    train_vs_artifact.argmax_match,
                    comparison_passes(train_vs_artifact, policy),
                )
                .with_train_vs_artifact_per_token_kl(train_vs_artifact.per_token_kl)
                .with_bundle_vs_artifact(
                    bundle_vs_artifact.max_abs_diff,
                    bundle_vs_artifact.argmax_match,
                )
                .with_bundle_vs_artifact_per_token_kl(bundle_vs_artifact.per_token_kl)
                .build()?
            }
        };
        records.push(record);
    }

    if records.is_empty() {
        return Err(AgreementError::NoRecordsForPhase {
            phase: policy.phase,
        });
    }
    Ok(records)
}

fn empty_record(
    seed: u64,
    prompt_id: impl Into<String>,
    checkpoint: SemanticCheckpoint,
    step: u32,
    phase: PhaseId,
    aggregation_kind: AggregationKind,
) -> AgreementRecord {
    AgreementRecord {
        seed,
        prompt_id: prompt_id.into(),
        checkpoint,
        step,
        phase,
        aggregation_kind,
        train_vs_bundle_max_abs_diff: None,
        train_vs_bundle_argmax_match: None,
        train_vs_bundle_per_token_kl: None,
        train_vs_bundle_pass: None,
        train_vs_artifact_max_abs_diff: None,
        train_vs_artifact_argmax_match: None,
        train_vs_artifact_per_token_kl: None,
        train_vs_artifact_pass: None,
        bundle_vs_artifact_max_abs_diff: None,
        bundle_vs_artifact_argmax_match: None,
        bundle_vs_artifact_per_token_kl: None,
    }
}

const fn is_agreement_gated_checkpoint(checkpoint: SemanticCheckpoint) -> bool {
    matches!(
        checkpoint,
        SemanticCheckpoint::PostLogits | SemanticCheckpoint::PostDecode
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct SurfaceComparison {
    max_abs_diff: f32,
    argmax_match: bool,
    per_token_kl: Option<f32>,
}

fn comparison_passes(comparison: SurfaceComparison, policy: AgreementPolicy) -> bool {
    comparison.max_abs_diff <= policy.max_logit_abs_diff
        && (!policy.require_argmax_token_match || comparison.argmax_match)
}

fn surface_comparison(
    left: &Observation,
    right: &Observation,
) -> Result<SurfaceComparison, AgreementError> {
    match (left, right) {
        (Observation::PostLogits { logits: left }, Observation::PostLogits { logits: right }) => {
            if left.len() != VOCAB_SIZE || right.len() != VOCAB_SIZE || left.len() != right.len() {
                return Err(AgreementError::LogitLengthMismatch {
                    left: left.len(),
                    right: right.len(),
                });
            }
            let max_abs_diff = left
                .iter()
                .zip(right)
                .map(|(left, right)| (left - right).abs())
                .fold(0.0_f32, f32::max);
            Ok(SurfaceComparison {
                max_abs_diff,
                argmax_match: argmax_lowest_index(left) == argmax_lowest_index(right),
                per_token_kl: per_token_softmax_kl(left, right),
            })
        }
        (Observation::PostDecode { token: left }, Observation::PostDecode { token: right }) => {
            let argmax_match = left == right;
            Ok(SurfaceComparison {
                max_abs_diff: if argmax_match { 0.0 } else { 1.0 },
                argmax_match,
                per_token_kl: None,
            })
        }
        (Observation::PostEmbedding { .. }, Observation::PostEmbedding { .. }) => {
            Err(AgreementError::UnsupportedCheckpoint {
                checkpoint: SemanticCheckpoint::PostEmbedding,
            })
        }
        _ => Err(AgreementError::ObservationKindMismatch {
            left: left.checkpoint(),
            right: right.checkpoint(),
        }),
    }
}

fn argmax_lowest_index(values: &[f32]) -> usize {
    let mut best_index = 0_usize;
    let mut best_value = values[0];
    for (index, value) in values.iter().copied().enumerate().skip(1) {
        if value > best_value {
            best_index = index;
            best_value = value;
        }
    }
    best_index
}

fn per_token_softmax_kl(left: &[f32], right: &[f32]) -> Option<f32> {
    if zero_norm(left) || zero_norm(right) {
        return None;
    }
    let left_max = finite_max(left)?;
    let right_max = finite_max(right)?;
    let left_denominator = exp_denominator(left, left_max)?;
    let right_denominator = exp_denominator(right, right_max)?;
    let mut kl = 0.0_f64;
    for (left_logit, right_logit) in left.iter().zip(right) {
        let p_num = (f64::from(*left_logit) - left_max).exp();
        let q_num = (f64::from(*right_logit) - right_max).exp();
        if p_num == 0.0 {
            continue;
        }
        if q_num == 0.0 {
            return None;
        }
        let p = p_num / left_denominator;
        let q = q_num / right_denominator;
        kl += p * (p.ln() - q.ln());
    }
    if !kl.is_finite() {
        return None;
    }
    Some(kl.max(0.0) as f32)
}

fn zero_norm(values: &[f32]) -> bool {
    values
        .iter()
        .map(|value| {
            let value = f64::from(*value);
            value * value
        })
        .sum::<f64>()
        == 0.0
}

fn finite_max(values: &[f32]) -> Option<f64> {
    values
        .iter()
        .copied()
        .map(f64::from)
        .reduce(f64::max)
        .filter(|value| value.is_finite())
}

fn exp_denominator(values: &[f32], max: f64) -> Option<f64> {
    let denominator = values
        .iter()
        .map(|value| (f64::from(*value) - max).exp())
        .sum::<f64>();
    if denominator.is_finite() && denominator > 0.0 {
        Some(denominator)
    } else {
        None
    }
}

/// Errors produced while constructing agreement products.
#[derive(Debug)]
pub enum AgreementError {
    /// A policy used an invalid tolerance.
    InvalidTolerance {
        /// Observed tolerance.
        observed: f32,
    },
    /// Prompt-wide aggregation is not a valid S3 agreement comparator.
    InvalidAggregation {
        /// Observed aggregation.
        observed: AggregationKind,
    },
    /// Policy was passed to the wrong phase slot.
    PolicyPhaseMismatch {
        /// Expected phase.
        expected: PhaseId,
        /// Observed phase.
        observed: PhaseId,
    },
    /// Live observation variant did not match its key checkpoint.
    CheckpointMismatch {
        /// Checkpoint in the key.
        key: SemanticCheckpoint,
        /// Checkpoint encoded by the observation variant.
        observed: SemanticCheckpoint,
    },
    /// Duplicate live observation key.
    DuplicateTrainObservation {
        /// S3 seed.
        seed: u64,
        /// Phase.
        phase: PhaseId,
        /// Prompt id.
        prompt_id: String,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
    /// Duplicate agreement record key.
    DuplicateRecord {
        /// S3 seed.
        seed: u64,
        /// Phase.
        phase: PhaseId,
        /// Prompt id.
        prompt_id: String,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
    /// No gated records were produced for a phase.
    NoRecordsForPhase {
        /// Phase.
        phase: PhaseId,
    },
    /// Product had no records.
    EmptyRecords,
    /// A phase-specific optional-field invariant was violated.
    OptionalFieldDiscipline {
        /// Phase.
        phase: PhaseId,
    },
    /// A record max-diff field was invalid.
    InvalidRecordDiff {
        /// Field name.
        name: &'static str,
        /// Observed value.
        value: f32,
    },
    /// Bundle observation was absent.
    MissingBundleObservation {
        /// Prompt id.
        prompt_id: String,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
    /// Artifact observation was absent.
    MissingArtifactObservation {
        /// Prompt id.
        prompt_id: String,
        /// Checkpoint.
        checkpoint: SemanticCheckpoint,
        /// Step.
        step: u32,
    },
    /// Two observations had incompatible checkpoint variants.
    ObservationKindMismatch {
        /// Left observation variant.
        left: SemanticCheckpoint,
        /// Right observation variant.
        right: SemanticCheckpoint,
    },
    /// Checkpoint is carried as observation-only and cannot be agreement-gated.
    UnsupportedCheckpoint {
        /// Unsupported checkpoint.
        checkpoint: SemanticCheckpoint,
    },
    /// Logit rows had incompatible lengths.
    LogitLengthMismatch {
        /// Left length.
        left: usize,
        /// Right length.
        right: usize,
    },
    /// Live-observation source metadata was internally inconsistent.
    InvalidLiveObservationSource {
        /// Reason.
        reason: &'static str,
    },
    /// Canonical JSON encoding failed.
    CanonicalJson(CanonicalJsonError),
}

impl fmt::Display for AgreementError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTolerance { observed } => {
                write!(
                    f,
                    "agreement tolerance must be finite and non-negative, got {observed}"
                )
            }
            Self::InvalidAggregation { observed } => write!(
                f,
                "S3 agreement requires per-token/per-vocab-row aggregation, got {observed:?}"
            ),
            Self::PolicyPhaseMismatch { expected, observed } => {
                write!(f, "expected policy for {expected:?}, got {observed:?}")
            }
            Self::CheckpointMismatch { key, observed } => {
                write!(
                    f,
                    "observation for {observed:?} cannot be stored at {key:?}"
                )
            }
            Self::DuplicateTrainObservation {
                seed,
                phase,
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "duplicate live observation for seed {seed}, {phase:?}, {prompt_id}, {checkpoint:?}, step {step}"
            ),
            Self::DuplicateRecord {
                seed,
                phase,
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "duplicate agreement record for seed {seed}, {phase:?}, {prompt_id}, {checkpoint:?}, step {step}"
            ),
            Self::NoRecordsForPhase { phase } => {
                write!(f, "no agreement records were produced for {phase:?}")
            }
            Self::EmptyRecords => f.write_str("agreement product requires at least one record"),
            Self::OptionalFieldDiscipline { phase } => {
                write!(
                    f,
                    "agreement optional-field discipline violated for {phase:?}"
                )
            }
            Self::InvalidRecordDiff { name, value } => {
                write!(f, "{name} must be finite and non-negative, got {value}")
            }
            Self::MissingBundleObservation {
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "missing bundle observation for {prompt_id}, {checkpoint:?}, step {step}"
            ),
            Self::MissingArtifactObservation {
                prompt_id,
                checkpoint,
                step,
            } => write!(
                f,
                "missing artifact observation for {prompt_id}, {checkpoint:?}, step {step}"
            ),
            Self::ObservationKindMismatch { left, right } => {
                write!(f, "cannot compare {left:?} observation against {right:?}")
            }
            Self::UnsupportedCheckpoint { checkpoint } => {
                write!(f, "{checkpoint:?} is not an agreement-gated checkpoint")
            }
            Self::LogitLengthMismatch { left, right } => {
                write!(f, "cannot compare logits of lengths {left} and {right}")
            }
            Self::InvalidLiveObservationSource { reason } => {
                write!(f, "invalid live-observation source: {reason}")
            }
            Self::CanonicalJson(error) => write!(f, "failed to encode agreement product: {error}"),
        }
    }
}

impl Error for AgreementError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CanonicalJson(error) => Some(error),
            Self::InvalidTolerance { .. }
            | Self::InvalidAggregation { .. }
            | Self::PolicyPhaseMismatch { .. }
            | Self::CheckpointMismatch { .. }
            | Self::DuplicateTrainObservation { .. }
            | Self::DuplicateRecord { .. }
            | Self::NoRecordsForPhase { .. }
            | Self::EmptyRecords
            | Self::OptionalFieldDiscipline { .. }
            | Self::InvalidRecordDiff { .. }
            | Self::MissingBundleObservation { .. }
            | Self::MissingArtifactObservation { .. }
            | Self::ObservationKindMismatch { .. }
            | Self::UnsupportedCheckpoint { .. }
            | Self::LogitLengthMismatch { .. }
            | Self::InvalidLiveObservationSource { .. } => None,
        }
    }
}
