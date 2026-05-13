//! Structured logging helpers for S2 loss composition.

use gbf_train::loss::composer::{ComposedLoss, InertClassification, LossTerms};

use crate::S2_LOG_TARGET;

/// Emit the S2 loss composition event and per-term inert classifications.
pub fn emit_loss_compose_events(
    step: u64,
    phase: &str,
    terms: &LossTerms,
    composed: &ComposedLoss,
) {
    tracing::debug!(
        target: S2_LOG_TARGET,
        event_name = "loss_compose",
        step,
        phase,
        lm_loss_nats = terms.lm_loss_next_byte_nats,
        total_loss = composed.total_loss,
        has_distill = terms.distill_loss_raw_nats.is_some(),
        has_range = terms.range_loss_raw.is_some(),
        has_zero = terms.zero_loss_raw.is_some(),
    );

    if !(0.0..=20.0).contains(&composed.total_loss) {
        tracing::warn!(
            target: S2_LOG_TARGET,
            event_name = "loss_compose_anomaly",
            step,
            total_loss = composed.total_loss,
            expected_band = "[0,20]",
        );
    }

    emit_classification(step, "distill", composed.inert_classification.distill);
    emit_classification(step, "balance", composed.inert_classification.balance);
    emit_classification(step, "zrouter", composed.inert_classification.zrouter);
    emit_classification(step, "switch", composed.inert_classification.switch);
    emit_classification(step, "range", composed.inert_classification.range);
    emit_classification(step, "zero", composed.inert_classification.zero);
    emit_classification(step, "shape", composed.inert_classification.shape);
    emit_classification(step, "overflow", composed.inert_classification.overflow);
}

fn emit_classification(step: u64, term: &str, classification: InertClassification) {
    match classification {
        InertClassification::ComputedDisabled { raw, weighted } => {
            tracing::debug!(
                target: S2_LOG_TARGET,
                event_name = "loss_term_classify",
                step,
                term,
                class = class_name(classification),
                raw,
                weighted,
            );
        }
        InertClassification::StructurallyInert => {
            tracing::debug!(
                target: S2_LOG_TARGET,
                event_name = "loss_term_classify",
                step,
                term,
                class = class_name(classification),
            );
        }
        InertClassification::Enabled { raw, weighted } => {
            tracing::debug!(
                target: S2_LOG_TARGET,
                event_name = "loss_term_classify",
                step,
                term,
                class = class_name(classification),
                raw,
                weighted,
            );
        }
    }
}

fn class_name(classification: InertClassification) -> &'static str {
    match classification {
        InertClassification::ComputedDisabled { .. } => "ComputedDisabled",
        InertClassification::StructurallyInert => "StructurallyInert",
        InertClassification::Enabled { .. } => "Enabled",
    }
}
