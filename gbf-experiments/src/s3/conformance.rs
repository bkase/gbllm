//! S3 conformance envelope helpers.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use std::path::Path;

use gbf_artifact::{
    AggregationKind, ConformanceEnvelope, EnvelopeGate, MetricGate, MetricId,
    QuantizationGapSummary, SeedConformanceEnvelope, canonical_conformance_bytes,
};
use gbf_foundation::{Hash256, sha256};
use gbf_oracle::denotational::SemanticCheckpoint as AgreementCheckpoint;
use gbf_oracle::phase_surface_agreement::{AgreementProduct, AgreementRecord, PhaseId};
use gbf_workload::WorkloadManifest_v0;

/// Tracing target for S3 conformance emission.
pub const CONFORMANCE_LOG_TARGET: &str = "gbf_experiments::s3::conformance";

/// Real conformance-envelope owner bead named by the S3 emission rule.
pub const CONFORMANCE_REAL_OWNER_BEAD: &str = gbf_artifact::CONFORMANCE_REAL_OWNER_BEAD;

/// Default emission path for `s3_conformance.v1`.
pub const S3_CONFORMANCE_EMISSION_PATH: &str = "experiments/S3/conformance/conformance.json";

/// Conformance build-started event name.
pub const EVENT_NAME_BUILD_STARTED: &str = "s3::conformance::build_started";
/// Per-seed envelope built event name.
pub const EVENT_NAME_SEED_ENVELOPE_BUILT: &str = "s3::conformance::seed_envelope_built";
/// Forbidden aggregation marker observed event name.
pub const EVENT_NAME_AGGREGATION_REJECTED: &str = "s3::conformance::aggregation_rejected";
/// Conformance build-complete event name.
pub const EVENT_NAME_BUILD_COMPLETE: &str = "s3::conformance::build_complete";

/// Build a single-workload `s3_conformance.v1` envelope from public agreement products.
pub fn build_conformance_envelope(
    workload: &WorkloadManifest_v0,
    agreement_products: Vec<AgreementProduct>,
) -> Result<ConformanceEnvelope, ConformanceError> {
    tracing::info!(
        target: CONFORMANCE_LOG_TARGET,
        event_name = EVENT_NAME_BUILD_STARTED,
        workload_self_hash = %workload.workload_self_hash,
        agreement_product_count = agreement_products.len() as u64,
        real_owner_bead = CONFORMANCE_REAL_OWNER_BEAD,
    );

    if agreement_products.is_empty() {
        return Err(ConformanceError::EmptyAgreementProducts);
    }

    let mut records_by_seed = BTreeMap::<u64, Vec<&AgreementRecord>>::new();
    for product in &agreement_products {
        for record in &product.records {
            records_by_seed.entry(record.seed).or_default().push(record);
        }
    }

    let mut per_seed = Vec::with_capacity(workload.seeds.len());
    let mut phase_a_gaps = Vec::new();
    let mut phase_d_gaps = Vec::new();
    let mut per_token_kls = Vec::new();
    for seed in &workload.seeds {
        let records = records_by_seed
            .get(seed)
            .ok_or(ConformanceError::MissingSeed { seed: *seed })?;
        let seed_envelope = build_seed_envelope(
            *seed,
            records,
            &mut phase_a_gaps,
            &mut phase_d_gaps,
            &mut per_token_kls,
        )?;
        tracing::trace!(
            target: CONFORMANCE_LOG_TARGET,
            event_name = EVENT_NAME_SEED_ENVELOPE_BUILT,
            seed = *seed,
            bundle_self_hash = %seed_envelope.bundle_self_hash,
            artifact_self_hash = %seed_envelope.artifact_self_hash,
            per_checkpoint_count = seed_envelope.per_checkpoint.len() as u64,
            per_metric_count = seed_envelope.per_metric.len() as u64,
        );
        per_seed.push(seed_envelope);
    }

    let overall_tolerance = per_seed
        .iter()
        .map(|seed| seed.overall.tolerance)
        .fold(0.0_f32, f32::max);
    let overall = EnvelopeGate {
        tolerance: overall_tolerance,
        passed: per_seed.iter().all(|seed| seed.overall.passed),
    };
    let quantization_gap_summary = QuantizationGapSummary {
        mean_per_token_max_abs_diff_phase_a: mean(&phase_a_gaps),
        mean_per_token_max_abs_diff_phase_d: mean(&phase_d_gaps),
        mean_per_token_kl: mean(&per_token_kls),
    };
    let envelope = ConformanceEnvelope::new(
        workload.workload_self_hash,
        per_seed,
        overall,
        quantization_gap_summary,
    )
    .map_err(ConformanceError::Artifact)?;

    tracing::info!(
        target: CONFORMANCE_LOG_TARGET,
        event_name = EVENT_NAME_BUILD_COMPLETE,
        per_seed_count = envelope.per_seed.len() as u64,
        overall_passed = envelope.overall.passed,
        conformance_self_hash = %envelope.conformance_self_hash,
        real_owner_bead = envelope.real_owner_bead.as_str(),
    );

    Ok(envelope)
}

/// Emit canonical `s3_conformance.v1` bytes to the default S3 path.
pub fn emit_default_conformance_json(
    envelope: &ConformanceEnvelope,
) -> Result<Vec<u8>, ConformanceError> {
    emit_conformance_json(S3_CONFORMANCE_EMISSION_PATH, envelope)
}

/// Emit canonical `s3_conformance.v1` bytes to `path`.
pub fn emit_conformance_json(
    path: impl AsRef<Path>,
    envelope: &ConformanceEnvelope,
) -> Result<Vec<u8>, ConformanceError> {
    let bytes = canonical_conformance_bytes(envelope).map_err(ConformanceError::Artifact)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| ConformanceError::Io {
            path: parent.display().to_string(),
            source,
        })?;
    }
    std::fs::write(path, &bytes).map_err(|source| ConformanceError::Io {
        path: path.display().to_string(),
        source,
    })?;
    Ok(bytes)
}

fn build_seed_envelope(
    seed: u64,
    records: &[&AgreementRecord],
    phase_a_gaps: &mut Vec<f32>,
    phase_d_gaps: &mut Vec<f32>,
    per_token_kls: &mut Vec<f32>,
) -> Result<SeedConformanceEnvelope, ConformanceError> {
    if records.is_empty() {
        return Err(ConformanceError::EmptySeedRecords { seed });
    }

    let mut per_checkpoint = BTreeMap::new();
    let mut per_metric = BTreeMap::new();
    for checkpoint in [
        AgreementCheckpoint::PostLogits,
        AgreementCheckpoint::PostDecode,
    ] {
        let checkpoint_records = records
            .iter()
            .copied()
            .filter(|record| record.checkpoint == checkpoint)
            .collect::<Vec<_>>();
        if checkpoint_records.is_empty() {
            return Err(ConformanceError::MissingCheckpoint { seed, checkpoint });
        }
        let tolerance = checkpoint_records
            .iter()
            .copied()
            .filter_map(|r| gated_diff(r))
            .fold(0.0_f32, f32::max);
        let passed = checkpoint_records.iter().all(|record| {
            record.train_vs_bundle_pass.unwrap_or(true)
                && record.train_vs_artifact_pass.unwrap_or(true)
        });
        per_checkpoint.insert(
            artifact_checkpoint(checkpoint),
            EnvelopeGate { tolerance, passed },
        );
    }

    for record in records {
        insert_record_metrics(
            seed,
            record,
            &mut per_metric,
            phase_a_gaps,
            phase_d_gaps,
            per_token_kls,
        )?;
    }
    if !per_metric
        .keys()
        .any(|metric_id| metric_id.as_str().contains("max_abs_logit_diff"))
    {
        return Err(ConformanceError::MissingMaxAbsMetric { seed });
    }

    let overall_tolerance = per_checkpoint
        .values()
        .map(|gate| gate.tolerance)
        .fold(0.0_f32, f32::max);
    let overall = EnvelopeGate {
        tolerance: overall_tolerance,
        passed: per_checkpoint.values().all(|gate| gate.passed),
    };

    Ok(SeedConformanceEnvelope {
        seed,
        bundle_self_hash: seed_surface_hash(seed, records, b"bundle"),
        artifact_self_hash: seed_surface_hash(seed, records, b"artifact"),
        overall,
        per_checkpoint,
        per_metric,
    })
}

fn insert_record_metrics(
    seed: u64,
    record: &AgreementRecord,
    per_metric: &mut BTreeMap<MetricId, MetricGate>,
    phase_a_gaps: &mut Vec<f32>,
    phase_d_gaps: &mut Vec<f32>,
    per_token_kls: &mut Vec<f32>,
) -> Result<(), ConformanceError> {
    let checkpoint = record.checkpoint.as_str();
    let phase = record.phase.as_str();
    let aggregation_kind = record.aggregation_kind;

    if aggregation_kind == AggregationKind::PromptWideSoftmaxForbidden {
        let rejected_metric_id = format!(
            "{}.{}.{}.step-{}.max_abs_logit_diff",
            record.prompt_id, phase, checkpoint, record.step
        );
        tracing::warn!(
            target: CONFORMANCE_LOG_TARGET,
            event_name = EVENT_NAME_AGGREGATION_REJECTED,
            seed,
            prompt_id = record.prompt_id.as_str(),
            metric_id = rejected_metric_id.as_str(),
            aggregation_kind = ?aggregation_kind,
        );
    }

    match record.phase {
        PhaseId::PhaseA => {
            if let Some(value) = record.train_vs_bundle_max_abs_diff {
                insert_metric(
                    per_metric,
                    metric_id(
                        &record.prompt_id,
                        phase,
                        checkpoint,
                        record.step,
                        "max_abs_logit_diff",
                    )?,
                    value,
                    aggregation_kind,
                    record.train_vs_bundle_pass.unwrap_or(false),
                );
            }
        }
        PhaseId::PhaseD => {
            if let Some(value) = record.train_vs_artifact_max_abs_diff {
                insert_metric(
                    per_metric,
                    metric_id(
                        &record.prompt_id,
                        phase,
                        checkpoint,
                        record.step,
                        "max_abs_logit_diff",
                    )?,
                    value,
                    aggregation_kind,
                    record.train_vs_artifact_pass.unwrap_or(false),
                );
            }
        }
    }

    if let Some(value) = record.bundle_vs_artifact_max_abs_diff {
        if record.checkpoint == AgreementCheckpoint::PostLogits {
            match record.phase {
                PhaseId::PhaseA => phase_a_gaps.push(value),
                PhaseId::PhaseD => phase_d_gaps.push(value),
            }
        }
        insert_metric(
            per_metric,
            metric_id(
                &record.prompt_id,
                phase,
                checkpoint,
                record.step,
                "bundle_vs_artifact_max_abs_diff",
            )?,
            value,
            aggregation_kind,
            true,
        );
    }

    if let Some(value) = record.bundle_vs_artifact_per_token_kl {
        if record.checkpoint == AgreementCheckpoint::PostLogits {
            insert_metric(
                per_metric,
                metric_id(
                    &record.prompt_id,
                    phase,
                    checkpoint,
                    record.step,
                    "bundle_vs_artifact_per_token_kl",
                )?,
                value,
                aggregation_kind,
                true,
            );
            per_token_kls.push(value);
        }
    }

    if let Some(argmax_match) = record
        .train_vs_bundle_argmax_match
        .or(record.train_vs_artifact_argmax_match)
    {
        insert_metric(
            per_metric,
            metric_id(
                &record.prompt_id,
                phase,
                checkpoint,
                record.step,
                "argmax_match",
            )?,
            if argmax_match { 0.0 } else { 1.0 },
            aggregation_kind,
            argmax_match,
        );
    }

    Ok(())
}

fn insert_metric(
    per_metric: &mut BTreeMap<MetricId, MetricGate>,
    metric_id: MetricId,
    value: f32,
    aggregation_kind: AggregationKind,
    passed: bool,
) {
    per_metric.insert(
        metric_id,
        MetricGate {
            value,
            aggregation_kind,
            passed,
        },
    );
}

fn metric_id(
    prompt_id: &str,
    phase: &str,
    checkpoint: &str,
    step: u32,
    metric: &str,
) -> Result<MetricId, ConformanceError> {
    MetricId::new(format!(
        "{prompt_id}.{phase}.{checkpoint}.step-{step}.{metric}"
    ))
    .map_err(ConformanceError::MetricId)
}

fn gated_diff(record: &AgreementRecord) -> Option<f32> {
    record
        .train_vs_bundle_max_abs_diff
        .or(record.train_vs_artifact_max_abs_diff)
}

fn artifact_checkpoint(checkpoint: AgreementCheckpoint) -> gbf_artifact::SemanticCheckpoint {
    match checkpoint {
        AgreementCheckpoint::PostEmbedding => gbf_artifact::SemanticCheckpoint::PostEmbedding,
        AgreementCheckpoint::PostLogits => gbf_artifact::SemanticCheckpoint::PostLogits,
        AgreementCheckpoint::PostDecode => gbf_artifact::SemanticCheckpoint::PostDecode,
    }
}

fn seed_surface_hash(seed: u64, records: &[&AgreementRecord], domain: &[u8]) -> Hash256 {
    let mut bytes = Vec::from("gbf-experiments:s3-conformance:surface-hash:v1\0");
    bytes.extend_from_slice(domain);
    bytes.push(0);
    bytes.extend_from_slice(&seed.to_le_bytes());
    for record in records {
        bytes.extend_from_slice(record.prompt_id.as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(record.checkpoint.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(record.phase.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&record.step.to_le_bytes());
        bytes.extend_from_slice(
            &record
                .train_vs_bundle_max_abs_diff
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &record
                .train_vs_artifact_max_abs_diff
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &record
                .bundle_vs_artifact_max_abs_diff
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &record
                .train_vs_bundle_per_token_kl
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &record
                .train_vs_artifact_per_token_kl
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
        bytes.extend_from_slice(
            &record
                .bundle_vs_artifact_per_token_kl
                .unwrap_or(-1.0)
                .to_bits()
                .to_le_bytes(),
        );
    }
    sha256(bytes)
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f32>() / values.len() as f32
}

/// Errors produced while building or emitting S3 conformance.
#[derive(Debug)]
pub enum ConformanceError {
    /// No agreement products were supplied.
    EmptyAgreementProducts,
    /// No records existed for a required seed.
    MissingSeed {
        /// Missing seed.
        seed: u64,
    },
    /// A seed had no records.
    EmptySeedRecords {
        /// Seed.
        seed: u64,
    },
    /// A seed was missing a required checkpoint.
    MissingCheckpoint {
        /// Seed.
        seed: u64,
        /// Checkpoint.
        checkpoint: AgreementCheckpoint,
    },
    /// A seed had no max-absolute-difference metrics.
    MissingMaxAbsMetric {
        /// Seed.
        seed: u64,
    },
    /// Metric id construction failed.
    MetricId(gbf_artifact::ids::ArtifactPathError),
    /// Artifact-layer conformance encoding failed.
    Artifact(gbf_artifact::ConformanceError),
    /// File-system emission failed.
    Io {
        /// Path being written or created.
        path: String,
        /// Source error.
        source: std::io::Error,
    },
}

impl fmt::Display for ConformanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyAgreementProducts => f.write_str("no agreement products supplied"),
            Self::MissingSeed { seed } => write!(f, "missing agreement records for seed {seed}"),
            Self::EmptySeedRecords { seed } => write!(f, "seed {seed} had no records"),
            Self::MissingCheckpoint { seed, checkpoint } => {
                write!(f, "seed {seed} missing checkpoint {checkpoint:?}")
            }
            Self::MissingMaxAbsMetric { seed } => {
                write!(f, "seed {seed} missing max_abs_logit_diff metric")
            }
            Self::MetricId(error) => write!(f, "invalid conformance metric id: {error}"),
            Self::Artifact(error) => write!(f, "{error}"),
            Self::Io { path, source } => write!(f, "failed to write {path}: {source}"),
        }
    }
}

impl Error for ConformanceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MetricId(error) => Some(error),
            Self::Artifact(error) => Some(error),
            Self::Io { source, .. } => Some(source),
            Self::EmptyAgreementProducts
            | Self::MissingSeed { .. }
            | Self::EmptySeedRecords { .. }
            | Self::MissingCheckpoint { .. }
            | Self::MissingMaxAbsMetric { .. } => None,
        }
    }
}
