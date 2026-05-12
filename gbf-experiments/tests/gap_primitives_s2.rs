mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::gap::{
    S2GapError, gap_ternary_vs_fp, try_gap_nodistill_vs_fp, try_gap_ternary_vs_fp,
};
use gbf_experiments::s2::schema::{
    S2BuildKind, S2ScoreReport, ScaleStatsSummary, ThresholdStatsSummary,
};
use gbf_foundation::Hash256;
use proptest::prelude::*;
use serde_json::{Value, json};

#[test]
fn bd_1btw_identical_scores_have_zero_gap_per_seed() {
    let ternary = score_array(S2BuildKind::s2_ternary_full, |seed| {
        2.0 + seed as f64 * 0.01
    });
    let fp = score_array(S2BuildKind::s2_fp_full, |seed| 2.0 + seed as f64 * 0.01);
    let capture = TraceCapture::default();

    let gaps = with_trace_capture(&capture, || gap_ternary_vs_fp(&ternary, &fp));

    assert_eq!(gaps, [0.0; 5]);
    let events = captured_events(&capture);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.name == "gap_ternary_vs_fp_per_seed")
            .count(),
        5
    );
    assert!(events.iter().any(|event| {
        event.name == "gap_aggregate"
            && event.fields.get("build").and_then(Value::as_str) == Some("ternary_vs_fp")
            && event.fields.get("median") == Some(&json!(0.0))
    }));
}

#[test]
fn bd_1btw_mismatched_seed_indexing_logs_alignment_error() {
    let ternary = score_array(S2BuildKind::s2_ternary_full, |_| 2.0);
    let fp = std::array::from_fn(|index| score(index as u64 + 1, S2BuildKind::s2_fp_full, 2.0));
    let capture = TraceCapture::default();

    let error =
        with_trace_capture(&capture, || try_gap_ternary_vs_fp(&ternary, &fp)).expect_err("gap");

    assert_eq!(
        error,
        S2GapError::SeedAlignment {
            index: 0,
            expected_seed: 0,
            got_seed: 1,
        }
    );
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "gap_seed_alignment_error"
            && event.fields.get("expected_seed") == Some(&json!(0))
            && event.fields.get("got_seed") == Some(&json!(1))
    }));
}

#[test]
fn bd_766b_build_kind_mismatch_is_logged_and_rejected() {
    let ternary = score_array(S2BuildKind::s2_fp_full, |_| 2.0);
    let fp = score_array(S2BuildKind::s2_fp_full, |_| 1.5);
    let capture = TraceCapture::default();

    let error =
        with_trace_capture(&capture, || try_gap_ternary_vs_fp(&ternary, &fp)).expect_err("gap");

    assert_eq!(
        error,
        S2GapError::BuildKindMismatch {
            seed: 0,
            expected: S2BuildKind::s2_ternary_full,
            got: S2BuildKind::s2_fp_full,
        }
    );
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "gap_build_kind_mismatch"
            && event.fields.get("seed") == Some(&json!(0))
            && event.fields.get("expected").and_then(Value::as_str) == Some("s2_ternary_full")
            && event.fields.get("got").and_then(Value::as_str) == Some("s2_fp_full")
    }));
}

#[test]
fn bd_1btw_non_finite_bpc_input_logs_and_hard_errors() {
    let mut ternary = score_array(S2BuildKind::s2_ternary_full, |_| 2.0);
    let fp = score_array(S2BuildKind::s2_fp_full, |_| 1.5);
    ternary[2].bpc = f64::NAN;
    let capture = TraceCapture::default();

    let error =
        with_trace_capture(&capture, || try_gap_ternary_vs_fp(&ternary, &fp)).expect_err("gap");

    assert!(matches!(
        error,
        S2GapError::NonFiniteBpc {
            seed: 2,
            ternary_bpc,
            fp_bpc: 1.5,
        } if ternary_bpc.is_nan()
    ));
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "gap_non_finite" && event.fields.get("seed") == Some(&json!(2))
    }));
}

#[test]
fn bd_1btw_gap_values_round_trip_as_f64_canonical_json() {
    let nodistill = score_array(S2BuildKind::s2_ternary_nodistill, |seed| {
        2.0 + seed as f64 * 0.125
    });
    let fp = score_array(S2BuildKind::s2_fp_full, |seed| 1.75 + seed as f64 * 0.0625);

    let gaps = try_gap_nodistill_vs_fp(&nodistill, &fp).expect("gap");
    let table = json!({
        "schema": "s2_report.v1",
        "build": "nodistill_vs_fp",
        "gap_bpc": gaps,
    });
    let bytes = S1CanonicalJson::value_to_vec(&table).expect("canonical table");
    let decoded: Value = serde_json::from_slice(&bytes).expect("table JSON");
    let encoded_again = S1CanonicalJson::value_to_vec(&decoded).expect("canonical table again");

    assert_eq!(encoded_again, bytes);
    assert_eq!(decoded["gap_bpc"], json!(gaps));
    insta::assert_snapshot!(
        "gap_s2__five_seed_table",
        serde_json::to_string_pretty(&decoded).expect("pretty JSON")
    );
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 128,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn bd_1btw_gap_arithmetic_preserves_f64_precision(
        pairs in prop::array::uniform5((0.0_f64..10_000.0, 0.0_f64..10_000.0)),
    ) {
        let ternary = std::array::from_fn(|index| {
            score(index as u64, S2BuildKind::s2_ternary_full, pairs[index].0)
        });
        let fp = std::array::from_fn(|index| {
            score(index as u64, S2BuildKind::s2_fp_full, pairs[index].1)
        });

        let gaps = try_gap_ternary_vs_fp(&ternary, &fp).expect("gap");

        for (index, gap) in gaps.iter().copied().enumerate() {
            prop_assert_eq!(gap.to_bits(), (pairs[index].0 - pairs[index].1).to_bits());
        }
    }
}

fn score_array(build_kind: S2BuildKind, bpc_for_seed: impl Fn(u64) -> f64) -> [S2ScoreReport; 5] {
    std::array::from_fn(|index| {
        let seed = index as u64;
        score(seed, build_kind, bpc_for_seed(seed))
    })
}

fn score(seed: u64, build_kind: S2BuildKind, bpc: f64) -> S2ScoreReport {
    S2ScoreReport::new(
        seed,
        build_kind,
        hash(1 + seed as u8),
        hash(20),
        1,
        bpc,
        needs_qat_stats(build_kind).then_some(threshold_stats()),
        needs_qat_stats(build_kind).then_some(scale_stats()),
    )
    .expect("score")
}

fn needs_qat_stats(build_kind: S2BuildKind) -> bool {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
}

fn threshold_stats() -> ThresholdStatsSummary {
    ThresholdStatsSummary {
        matrices: 2,
        threshold_min: 0.1,
        threshold_max: 0.4,
        threshold_mean: 0.25,
        threshold_count: 4,
    }
}

fn scale_stats() -> ScaleStatsSummary {
    ScaleStatsSummary {
        matrices: 2,
        scale_count: 4,
        scale_min: 1.0,
        scale_max: 2.5,
        scale_mean_f32: 1.75,
    }
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
