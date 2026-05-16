//! Canonical write boundary for `s3_conformance.v1`.

use crate::{AggregationKind, ConformanceEnvelope, ConformanceError, MetricId, SemanticCheckpoint};

/// Encoder for the `s3_conformance.v1` artifact write boundary.
#[derive(Debug, Clone, Copy)]
pub struct CanonicalConformanceWrite;

impl CanonicalConformanceWrite {
    /// Encode a conformance envelope after enforcing S3 write invariants.
    pub fn to_vec(envelope: &ConformanceEnvelope) -> Result<Vec<u8>, ConformanceError> {
        validate_canonical_conformance_write(envelope)?;
        envelope.canonical_bytes_unchecked()
    }
}

/// Encode a conformance envelope at the canonical write boundary.
pub fn canonical_conformance_bytes(
    envelope: &ConformanceEnvelope,
) -> Result<Vec<u8>, ConformanceError> {
    CanonicalConformanceWrite::to_vec(envelope)
}

fn validate_canonical_conformance_write(
    envelope: &ConformanceEnvelope,
) -> Result<(), ConformanceError> {
    for seed in &envelope.per_seed {
        for checkpoint in [
            SemanticCheckpoint::PostLogits,
            SemanticCheckpoint::PostDecode,
        ] {
            if !seed.per_checkpoint.contains_key(&checkpoint) {
                return Err(ConformanceError::MissingCheckpoint {
                    seed: seed.seed,
                    checkpoint,
                });
            }
        }

        if !seed
            .per_metric
            .keys()
            .any(|metric_id| metric_id.as_str().contains("max_abs_logit_diff"))
        {
            return Err(ConformanceError::MissingMetric {
                seed: seed.seed,
                metric_name: "max_abs_logit_diff",
            });
        }

        if let Some(metric_id) = forbidden_metric(seed, true) {
            return Err(ConformanceError::PromptWideSoftmaxAggregation {
                seed: seed.seed,
                prompt_id: prompt_id_from_metric_id(metric_id),
                metric_id: metric_id.clone(),
            });
        }
        if let Some(metric_id) = forbidden_metric(seed, false) {
            return Err(ConformanceError::PromptWideSoftmaxAggregation {
                seed: seed.seed,
                prompt_id: prompt_id_from_metric_id(metric_id),
                metric_id: metric_id.clone(),
            });
        }
    }

    let max_seed_tolerance = envelope
        .per_seed
        .iter()
        .map(|seed| seed.overall.tolerance)
        .fold(0.0_f32, f32::max);
    if envelope.overall.tolerance < max_seed_tolerance {
        return Err(ConformanceError::OverallToleranceTooStrict {
            overall: envelope.overall.tolerance,
            max_seed_tolerance,
        });
    }

    let recomputed = envelope.compute_self_hash()?;
    if recomputed != envelope.conformance_self_hash {
        return Err(ConformanceError::SelfHashMismatch {
            expected: recomputed,
            observed: envelope.conformance_self_hash,
        });
    }

    Ok(())
}

fn forbidden_metric(
    seed: &crate::SeedConformanceEnvelope,
    require_max_abs_logit_diff: bool,
) -> Option<&MetricId> {
    seed.per_metric
        .iter()
        .find(|(metric_id, metric)| {
            metric.aggregation_kind == AggregationKind::PromptWideSoftmaxForbidden
                && (!require_max_abs_logit_diff
                    || metric_id.as_str().contains("max_abs_logit_diff"))
        })
        .map(|(metric_id, _)| metric_id)
}

fn prompt_id_from_metric_id(metric_id: &MetricId) -> String {
    metric_id
        .as_str()
        .split('.')
        .next()
        .unwrap_or("unknown_prompt")
        .to_owned()
}
