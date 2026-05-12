mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::schema::{
    HypothesisStatus, S2BuildKind, S2ScoreReport, ScaleStatsSummary, ThresholdStatsSummary,
};
use gbf_experiments::s2::verifiers::{DiagnosticHit, verify_h2};
use gbf_foundation::Hash256;
use proptest::prelude::*;
use serde_json::Value;

#[test]
fn h2_clean_scores_confirm() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 1.7, 1.8, 1.9, 2.0]);
    let fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 1.6, 1.7, 1.8]);
    let capture = TraceCapture::default();

    let verdict = with_trace_capture(&capture, || verify_h2(&ternary, &fp));

    assert_eq!(verdict.status, HypothesisStatus::Confirmed);
    assert!(verdict.hits.is_empty());
    assert!(
        verdict
            .gap
            .expect("validated H2 inputs should produce gaps")
            .iter()
            .all(|gap| *gap <= 0.5)
    );
    assert!(captured_events(&capture).iter().any(|event| {
        event.name == "h2_verdict"
            && event.fields.get("status").and_then(Value::as_str) == Some("Confirmed")
    }));
}

#[test]
fn h2_input_validation_failure_has_no_gap_vector() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 1.7, 1.8, 1.9, 2.0]);
    let mut fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 1.6, 1.7, 1.8]);
    fp[2].seed = 99;

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_eq!(verdict.gap, None);
    assert_hit(&verdict.hits, "gap_inputs", Some(2));
}

#[test]
fn h2_refutes_one_seed_gap_violation() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 2.2, 1.8, 1.9, 2.0]);
    let fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 1.6, 1.7, 1.8]);

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "per_seed_gap", Some(1));
}

#[test]
fn h2_refutes_fp_quality_violation() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 1.7, 2.8, 1.9, 2.0]);
    let fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 2.6, 1.7, 1.8]);

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "fp_quality", Some(2));
}

#[test]
fn h2_quality_hit_snapshot_pins_shape() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 1.7, 2.8, 1.9, 2.0]);
    let fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 2.6, 1.7, 1.8]);

    let verdict = verify_h2(&ternary, &fp);

    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "h2_diagnostic_hit__quality_sanity_violation",
            pretty_json(&serde_json::to_value(first_hit(&verdict.hits, "fp_quality")).unwrap())
        );
    });
}

#[test]
fn h2_refutes_ternary_quality_violation() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [1.6, 1.7, 1.8, 3.1, 2.0]);
    let fp = scores(S2BuildKind::s2_fp_full, [1.4, 1.5, 1.6, 2.7, 1.8]);

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "ternary_quality", Some(3));
}

#[test]
fn h2_refutes_suspicious_low_median_fp() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [0.6, 0.7, 0.8, 0.8, 0.8]);
    let fp = scores(S2BuildKind::s2_fp_full, [0.3, 0.4, 0.4, 0.45, 0.49]);

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "suspicious_low_median_fp", None);
}

#[test]
fn h2_refutes_suspicious_low_median_ternary() {
    let ternary = scores(S2BuildKind::s2_ternary_full, [0.3, 0.4, 0.4, 0.45, 0.49]);
    let fp = scores(S2BuildKind::s2_fp_full, [0.6, 0.7, 0.7, 0.75, 0.8]);

    let verdict = verify_h2(&ternary, &fp);

    assert_eq!(verdict.status, HypothesisStatus::Refuted);
    assert_hit(&verdict.hits, "suspicious_low_median_ternary", None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn h2_score_sets_within_bounds_confirm(
        fp_bpcs in proptest::array::uniform5(1.4_f64..=1.9),
        gaps in proptest::array::uniform5(0.0_f64..=0.4),
    ) {
        let mut ternary_bpcs = [0.0; 5];
        for index in 0..5 {
            ternary_bpcs[index] = fp_bpcs[index] + gaps[index];
        }
        let ternary = scores(S2BuildKind::s2_ternary_full, ternary_bpcs);
        let fp = scores(S2BuildKind::s2_fp_full, fp_bpcs);

        let verdict = verify_h2(&ternary, &fp);

        prop_assert_eq!(verdict.status, HypothesisStatus::Confirmed);
        prop_assert!(verdict.hits.is_empty());
    }
}

fn scores(build_kind: S2BuildKind, bpcs: [f64; 5]) -> [S2ScoreReport; 5] {
    std::array::from_fn(|index| score(index as u64, build_kind, bpcs[index]))
}

fn score(seed: u64, build_kind: S2BuildKind, bpc: f64) -> S2ScoreReport {
    S2ScoreReport::new(
        seed,
        build_kind,
        hash(1),
        hash(2),
        1,
        bpc,
        qat_threshold_stats(build_kind),
        qat_scale_stats(build_kind),
    )
    .expect("score report")
}

fn qat_threshold_stats(build_kind: S2BuildKind) -> Option<ThresholdStatsSummary> {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
    .then_some(ThresholdStatsSummary {
        matrices: 1,
        threshold_min: 0.2,
        threshold_max: 0.4,
        threshold_mean: 0.3,
        threshold_count: 4,
    })
}

fn qat_scale_stats(build_kind: S2BuildKind) -> Option<ScaleStatsSummary> {
    matches!(
        build_kind,
        S2BuildKind::s2_ternary_full | S2BuildKind::s2_ternary_nodistill
    )
    .then_some(ScaleStatsSummary {
        matrices: 1,
        scale_count: 4,
        scale_min: 0.5,
        scale_max: 1.5,
        scale_mean_f32: 1.0,
    })
}

fn assert_hit(hits: &[DiagnosticHit], check_name: &str, seed: Option<u64>) {
    assert!(
        hits.iter()
            .any(|hit| hit.check_name == check_name && hit.seed == seed),
        "missing hit {check_name} seed {seed:?}: {hits:#?}"
    );
}

fn first_hit<'a>(hits: &'a [DiagnosticHit], check_name: &str) -> &'a DiagnosticHit {
    hits.iter()
        .find(|hit| hit.check_name == check_name)
        .unwrap_or_else(|| panic!("missing hit {check_name}: {hits:#?}"))
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("snapshot value serializes")
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
