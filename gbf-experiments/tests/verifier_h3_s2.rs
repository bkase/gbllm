mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::schema::HypothesisStatus;
use gbf_experiments::s2::verifiers::{H3Score, verify_h3};
use serde_json::{Value, json};

#[test]
fn h3_distillation_helps_every_seed_confirms() {
    let capture = TraceCapture::default();
    let verification = with_trace_capture(&capture, || {
        verify_h3(
            &[
                H3Score { seed: 0, bpc: 1.10 },
                H3Score { seed: 1, bpc: 1.20 },
                H3Score { seed: 2, bpc: 1.15 },
            ],
            &[
                H3Score { seed: 0, bpc: 1.00 },
                H3Score { seed: 1, bpc: 1.00 },
                H3Score { seed: 2, bpc: 1.00 },
            ],
            &[
                H3Score { seed: 0, bpc: 1.30 },
                H3Score { seed: 1, bpc: 1.50 },
                H3Score { seed: 2, bpc: 1.40 },
            ],
        )
        .unwrap()
    });

    assert_eq!(verification.status, HypothesisStatus::Confirmed);
    assert!(verification.weak_form_passed);
    assert_close(verification.strong_form_observed.unwrap(), 0.25);
    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "h3_per_seed")
            .count(),
        3
    );
    assert!(events.iter().any(|event| {
        event.name == "h3_verdict"
            && event.fields.get("status").and_then(Value::as_str) == Some("Confirmed")
    }));

    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!("h3_per_seed__weak_form_pass", pretty_json(&h3_snapshot(&verification)));
    });
}

#[test]
fn h3_refutes_when_one_seed_exceeds_weak_tolerance() {
    let capture = TraceCapture::default();
    let verification = with_trace_capture(&capture, || {
        verify_h3(
            &[H3Score { seed: 7, bpc: 1.50 }],
            &[H3Score { seed: 7, bpc: 1.00 }],
            &[H3Score { seed: 7, bpc: 1.30 }],
        )
        .unwrap()
    });

    assert_eq!(verification.status, HypothesisStatus::Refuted);
    assert!(!verification.weak_form_passed);
    assert_close(verification.strong_form_observed.unwrap(), -0.2);
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h3_per_seed"
            && event.fields.get("seed").and_then(Value::as_u64) == Some(7)
            && event.fields.get("passes").and_then(Value::as_bool) == Some(false)
    }));
}

#[test]
fn h3_missing_nodistill_control_is_prior_gate() {
    let verification = verify_h3(
        &[H3Score { seed: 0, bpc: 1.10 }],
        &[H3Score { seed: 0, bpc: 1.00 }],
        &[],
    )
    .unwrap();

    assert_eq!(
        verification.status,
        HypothesisStatus::NotEvaluatedDueToPriorGate {
            reason: "nodistill control absent".to_owned()
        }
    );
    assert!(verification.per_seed.is_empty());
}

#[test]
fn h3_strong_form_is_recorded_even_when_weak_form_refutes() {
    let verification = verify_h3(
        &[
            H3Score { seed: 0, bpc: 1.05 },
            H3Score { seed: 1, bpc: 1.55 },
        ],
        &[
            H3Score { seed: 0, bpc: 1.00 },
            H3Score { seed: 1, bpc: 1.00 },
        ],
        &[
            H3Score { seed: 0, bpc: 1.20 },
            H3Score { seed: 1, bpc: 1.30 },
        ],
    )
    .unwrap();

    assert_eq!(verification.status, HypothesisStatus::Refuted);
    assert_close(verification.strong_form_observed.unwrap(), -0.05);
}

fn h3_snapshot(verification: &gbf_experiments::s2::verifiers::H3Verification) -> Value {
    json!({
        "status": verification.status,
        "weak_form_passed": verification.weak_form_passed,
        "strong_form_observed": verification.strong_form_observed,
        "per_seed": verification.per_seed.iter().map(|entry| {
            json!({
                "seed": entry.seed,
                "gap_distill": entry.gap_distill,
                "gap_nodistill": entry.gap_nodistill,
                "passes": entry.passes,
            })
        }).collect::<Vec<_>>(),
    })
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot value serializes")
}

fn assert_close(observed: f64, expected: f64) {
    assert!(
        (observed - expected).abs() < 1.0e-12,
        "observed {observed}, expected {expected}"
    );
}
