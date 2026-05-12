mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::loss_logging::emit_loss_compose_events;
use gbf_train::loss::composer::{
    ComposeError, InertClassification, LossTermApplicability, LossTerms, PhaseEffectiveLossWeights,
    PhaseEffectiveLossWeightsValues, TrainingLossUnit, compose,
};
use serde_json::{Value, json};

#[test]
fn composer_sums_lm_and_distill_nats() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            distill_loss_raw_nats: Some(2.0),
            ..LossTerms::default()
        },
        weights(1.0, 0.0, 0.0),
        LossTermApplicability {
            distill: true,
            ..LossTermApplicability::toy0_phase_a_without_distill_call()
        },
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(composed.total_loss, 2.5);
    assert_eq!(composed.weighted.distill, Some(2.0));
}

#[test]
fn composer_rejects_non_nats_unit() {
    assert_eq!(
        compose(
            LossTerms {
                lm_loss_next_byte_nats: 0.5,
                ..LossTerms::default()
            },
            PhaseEffectiveLossWeights::zero(),
            LossTermApplicability::toy0_phase_a_without_distill_call(),
            TrainingLossUnit::unsupported("bpc"),
        )
        .unwrap_err(),
        ComposeError::UnitMismatch {
            got: "bpc".to_owned()
        }
    );
}

#[test]
fn composer_errors_on_missing_enabled_raw_loss() {
    assert_eq!(
        compose(
            LossTerms {
                lm_loss_next_byte_nats: 0.5,
                distill_loss_raw_nats: Some(0.0),
                zero_loss_raw: Some(0.0),
                ..LossTerms::default()
            },
            weights(0.0, 0.01, 0.0),
            LossTermApplicability::toy0_phase_cd(),
            TrainingLossUnit::Nats,
        )
        .unwrap_err(),
        ComposeError::MissingRawLoss { term: "range" }
    );
}

#[test]
fn loss_logging_emits_compose_and_term_classification_events() {
    let terms = LossTerms {
        lm_loss_next_byte_nats: 0.5,
        distill_loss_raw_nats: Some(2.5),
        range_loss_raw: Some(0.3),
        zero_loss_raw: Some(0.0),
        ..LossTerms::default()
    };
    let composed = compose(
        terms,
        weights(0.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        emit_loss_compose_events(8_100, "PhaseD", &terms, &composed);
    });

    let events = captured_events(&capture);
    assert!(events.iter().any(|event| event.name == "loss_compose"));
    assert!(events.iter().any(|event| {
        event.name == "loss_term_classify"
            && event.fields.get("term").and_then(serde_json::Value::as_str) == Some("distill")
            && event
                .fields
                .get("class")
                .and_then(serde_json::Value::as_str)
                == Some("ComputedDisabled")
    }));
}

#[test]
fn loss_logging_preserves_structurally_inert_null_semantics_in_event_shape() {
    let terms = LossTerms {
        lm_loss_next_byte_nats: 0.5,
        distill_loss_raw_nats: Some(2.5),
        range_loss_raw: Some(0.0),
        zero_loss_raw: Some(0.0),
        ..LossTerms::default()
    };
    let composed = compose(
        terms,
        weights(0.0, 0.01, 0.0),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        emit_loss_compose_events(8_200, "PhaseC", &terms, &composed);
    });

    let events = captured_events(&capture);
    let balance = loss_term_event(&events, "balance");
    assert_eq!(
        balance
            .fields
            .get("class")
            .and_then(serde_json::Value::as_str),
        Some("StructurallyInert")
    );
    assert!(!balance.fields.contains_key("raw"));
    assert!(!balance.fields.contains_key("weighted"));
    assert!(!balance.fields.contains_key("raw_present"));
    assert!(!balance.fields.contains_key("weighted_present"));

    let range = loss_term_event(&events, "range");
    assert_eq!(
        range
            .fields
            .get("class")
            .and_then(serde_json::Value::as_str),
        Some("Enabled")
    );
    assert_eq!(range.fields.get("raw"), Some(&json!(0.0)));
    assert_eq!(range.fields.get("weighted"), Some(&json!(0.0)));
}

#[test]
fn nonzero_lambda_on_structurally_inert_term_emits_warning() {
    let terms = LossTerms {
        lm_loss_next_byte_nats: 0.5,
        ..LossTerms::default()
    };
    let lambdas = PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_balance: 0.25,
        lambda_zrouter: 0.5,
        ..zero_values()
    })
    .unwrap();
    let capture = TraceCapture::default();

    let composed = with_trace_capture(&capture, || {
        compose(
            terms,
            lambdas,
            LossTermApplicability::toy0_phase_a_without_distill_call(),
            TrainingLossUnit::Nats,
        )
        .unwrap()
    });

    assert_eq!(
        composed.inert_classification.balance,
        InertClassification::StructurallyInert
    );
    let warnings = captured_events(&capture)
        .into_iter()
        .filter(|event| event.name == "loss_structurally_inert_nonzero_lambda")
        .collect::<Vec<_>>();
    assert_eq!(warnings.len(), 2);
    assert!(warnings.iter().all(|event| event.level == "WARN"));
    assert!(warnings.iter().any(|event| {
        event.fields.get("term").and_then(serde_json::Value::as_str) == Some("balance")
    }));
    assert!(warnings.iter().any(|event| {
        event.fields.get("term").and_then(serde_json::Value::as_str) == Some("zrouter")
    }));
}

#[test]
fn literal_zero_raw_is_allowed_when_enabled() {
    let composed = compose(
        LossTerms {
            lm_loss_next_byte_nats: 0.5,
            range_loss_raw: Some(0.0),
            ..LossTerms::default()
        },
        weights(0.0, 0.01, 0.0),
        LossTermApplicability {
            range: true,
            ..LossTermApplicability::toy0_phase_a_without_distill_call()
        },
        TrainingLossUnit::Nats,
    )
    .unwrap();

    assert_eq!(
        composed.inert_classification.range,
        InertClassification::Enabled {
            raw: 0.0,
            weighted: 0.0
        }
    );
}

#[test]
fn phase_a_step_snapshot_records_structurally_inert_nulls() {
    let terms = LossTerms {
        lm_loss_next_byte_nats: 0.5,
        ..LossTerms::default()
    };
    let composed = compose(
        terms,
        PhaseEffectiveLossWeights::zero(),
        LossTermApplicability::toy0_phase_a_without_distill_call(),
        TrainingLossUnit::Nats,
    )
    .unwrap();
    let report = compose_snapshot_report(0, "PhaseA", &terms, &composed);

    assert!(report["eval_points"]["distill"]["raw"].is_null());
    assert!(report["eval_points"]["distill"]["weighted"].is_null());
    insta::assert_snapshot!("phase_a_step", pretty_json(&report));
}

#[test]
fn phase_d_step_with_qat_snapshot_records_weighted_terms() {
    let terms = LossTerms {
        lm_loss_next_byte_nats: 0.5,
        distill_loss_raw_nats: Some(2.0),
        range_loss_raw: Some(0.3),
        zero_loss_raw: Some(0.2),
        ..LossTerms::default()
    };
    let composed = compose(
        terms,
        weights(1.0, 0.01, 0.0001),
        LossTermApplicability::toy0_phase_cd(),
        TrainingLossUnit::Nats,
    )
    .unwrap();
    let report = compose_snapshot_report(600, "PhaseD", &terms, &composed);

    assert_eq!(composed.total_loss, 2.50302);
    assert_eq!(report["eval_points"]["balance"]["raw"], Value::Null);
    insta::assert_snapshot!("phase_d_step_with_qat", pretty_json(&report));
}

fn weights(lambda_distill: f32, lambda_range: f32, lambda_zero: f32) -> PhaseEffectiveLossWeights {
    PhaseEffectiveLossWeights::new(PhaseEffectiveLossWeightsValues {
        lambda_distill,
        lambda_balance: 0.0,
        lambda_zrouter: 0.0,
        lambda_switch: 0.0,
        lambda_range,
        lambda_zero,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    })
    .unwrap()
}

fn zero_values() -> PhaseEffectiveLossWeightsValues {
    PhaseEffectiveLossWeightsValues {
        lambda_distill: 0.0,
        lambda_balance: 0.0,
        lambda_zrouter: 0.0,
        lambda_switch: 0.0,
        lambda_range: 0.0,
        lambda_zero: 0.0,
        lambda_shape: 0.0,
        lambda_overflow: 0.0,
    }
}

fn compose_snapshot_report(
    step: u64,
    phase: &str,
    terms: &LossTerms,
    composed: &gbf_train::loss::composer::ComposedLoss,
) -> Value {
    json!({
        "step": step,
        "phase": phase,
        "unit": TrainingLossUnit::Nats,
        "lm_loss_next_byte_nats": terms.lm_loss_next_byte_nats,
        "total_loss": composed.total_loss,
        "weighted": composed.weighted,
        "eval_points": composed.inert_classification.eval_points(),
    })
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot report serializes")
}

fn loss_term_event<'a>(
    events: &'a [common::tracing_capture::TracingEvent],
    term: &str,
) -> &'a common::tracing_capture::TracingEvent {
    events
        .iter()
        .find(|event| {
            event.name == "loss_term_classify"
                && event.fields.get("term").and_then(serde_json::Value::as_str) == Some(term)
        })
        .unwrap_or_else(|| panic!("missing loss_term_classify event for {term}"))
}
