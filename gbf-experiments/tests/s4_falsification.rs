#![cfg(feature = "s4-falsify")]

mod common;
#[path = "s4_falsification/common.rs"]
mod common_falsification;
#[path = "s4_falsification/s4_f1_lossy_decompression.rs"]
mod s4_f1_lossy_decompression;
#[path = "s4_falsification/s4_f2_window_too_small.rs"]
mod s4_f2_window_too_small;
#[path = "s4_falsification/s4_f3_gate_skips_oracle.rs"]
mod s4_f3_gate_skips_oracle;
#[path = "s4_falsification/s4_f4_train_random_init.rs"]
mod s4_f4_train_random_init;
#[path = "s4_falsification/s4_f5_oracle_drift.rs"]
mod s4_f5_oracle_drift;
#[path = "s4_falsification/s4_f6_unmappable_dropped.rs"]
mod s4_f6_unmappable_dropped;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use common_falsification::suite_inputs;
use gbf_experiments::s4::falsify::{
    S4_FALSIFY_OUTCOME_EVENT_NAME, S4_FALSIFY_VARIANT_RUN_EVENT_NAME, S4FalsificationCase,
    run_s4_falsification_suite,
};
use serde_json::json;

#[test]
fn complete_s4_falsification_suite_runs_all_six_broken_variants_in_o5_order() {
    let report = run_s4_falsification_suite(&suite_inputs());

    assert_eq!(S4FalsificationCase::ALL.len(), 6);
    assert!(report.passed(), "{report:?}");
    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.case)
            .collect::<Vec<_>>(),
        S4FalsificationCase::ALL
    );
    assert_eq!(
        report
            .results
            .iter()
            .map(|result| result.case.case_id())
            .collect::<Vec<_>>(),
        vec![
            "F1-broken-S4",
            "F2-broken-S4",
            "F3-broken-S4",
            "F4-broken-S4",
            "F5-broken-S4",
            "F6-broken-S4",
        ]
    );
}

#[test]
fn s4_falsification_suite_emits_variant_run_and_outcome_events() {
    let capture = TraceCapture::default();
    let report = with_trace_capture(&capture, || run_s4_falsification_suite(&suite_inputs()));

    assert!(report.passed(), "{report:?}");
    let events = captured_events(&capture);
    let variant_events = events
        .iter()
        .filter(|event| event.name == S4_FALSIFY_VARIANT_RUN_EVENT_NAME)
        .collect::<Vec<_>>();
    let outcome_events = events
        .iter()
        .filter(|event| event.name == S4_FALSIFY_OUTCOME_EVENT_NAME)
        .collect::<Vec<_>>();
    assert_eq!(variant_events.len(), S4FalsificationCase::ALL.len());
    assert_eq!(outcome_events.len(), S4FalsificationCase::ALL.len());

    for (idx, case) in S4FalsificationCase::ALL.into_iter().enumerate() {
        let variant = variant_events[idx];
        assert_eq!(variant.fields.get("case_id"), Some(&json!(case.case_id())));
        assert_eq!(variant.fields.get("variant"), Some(&json!(case.as_str())));
        assert_eq!(
            variant.fields.get("expected_refuted_hypothesis"),
            Some(&json!(expected_hypothesis_label(case)))
        );

        let outcome = outcome_events[idx];
        assert_eq!(outcome.fields.get("case_id"), Some(&json!(case.case_id())));
        assert_eq!(outcome.fields.get("variant"), Some(&json!(case.as_str())));
        assert_eq!(
            outcome.fields.get("expected_refuted_hypothesis"),
            Some(&json!(expected_hypothesis_label(case)))
        );
        assert_eq!(
            outcome.fields.get("observed_status"),
            Some(&json!("Refuted"))
        );
        assert_eq!(
            outcome.fields.get("refuted_as_expected"),
            Some(&json!(true))
        );
        assert!(
            outcome
                .fields
                .get("detail")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|detail| !detail.is_empty()),
            "outcome detail should be present for {case:?}: {outcome:?}"
        );
    }
}

fn expected_hypothesis_label(case: S4FalsificationCase) -> &'static str {
    match case {
        S4FalsificationCase::LossyGutenbergDecompression
        | S4FalsificationCase::UnmappableRateSilentlyDropped => "H1",
        S4FalsificationCase::ContaminationWindowTooSmall => "H2",
        S4FalsificationCase::PromotionGateSkipsOracleAgreement => "H3",
        S4FalsificationCase::TrainRandomInit => "H6",
        S4FalsificationCase::OracleDriftUnderCorpusSwitch => "H5",
    }
}
