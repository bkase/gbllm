mod common;

use std::collections::BTreeMap;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s2::schema::{
    DistillEvalPoint, DistillationLog, LossTermEvalPoint, PhaseEffectiveLambda,
    PhaseEffectiveLambdaValues, S2BuildKind, S2ScoreReport, ScaleStatsSummary,
    ThresholdStatsSummary, write_distillation_log, write_score_report,
};
use gbf_foundation::Hash256;

#[test]
fn score_report_validates_qat_stats_nullability_and_writes_canonical_json() {
    let ternary = ternary_score().expect("ternary score");
    let fp = S2ScoreReport::new(
        0,
        S2BuildKind::s2_fp_full,
        hash(1),
        hash(2),
        4_096,
        8_192.0,
        None,
        None,
    )
    .expect("fp score");
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("score.json");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_score_report(&path, &ternary).expect("score write")
    });

    assert_eq!(
        ternary.score_self_hash,
        ternary.computed_self_hash().expect("score hash")
    );
    assert!(fp.validate().is_ok());
    assert_eq!(
        std::fs::read(&path).expect("score file"),
        gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&ternary).expect("score json")
    );
    assert!(
        captured_events(&capture)
            .iter()
            .any(|event| event.name == "score_emitter_finalized")
    );
    let mut invalid = fp;
    invalid.threshold_stats = Some(threshold_stats());
    assert!(invalid.validate().is_err());
    insta::assert_snapshot!(
        "score_s2__ternary_seed0_tiny",
        String::from_utf8(ternary.canonical_json_bytes().expect("score canonical")).unwrap()
    );
}

#[test]
fn distillation_log_validates_eval_points_and_writer_event() {
    let log = nodistill_log().expect("distillation log");
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("distill-log.json");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        write_distillation_log(&path, &log).expect("distill write");
    });

    assert_eq!(
        log.distill_log_self_hash,
        log.computed_self_hash().expect("distill hash")
    );
    assert_eq!(
        std::fs::read(&path).expect("distill file"),
        gbf_experiments::s1::schema::S1CanonicalJson::to_vec(&log).expect("distill json")
    );
    assert!(
        captured_events(&capture)
            .iter()
            .any(|event| event.name == "distill_log_finalized")
    );
    insta::assert_snapshot!(
        "distill_log_s2__nodistill_phase_c",
        String::from_utf8(log.canonical_json_bytes().expect("distill canonical")).unwrap()
    );
}

#[test]
fn distillation_log_rejects_pre_phase_c_loss_and_misaligned_terms() {
    let mut log = nodistill_log().expect("distillation log");
    log.distill_loss_per_eval_point[0].distill_loss = Some(0.1);
    assert!(log.validate().is_err());

    let mut log = nodistill_log().expect("distillation log");
    log.loss_terms_per_eval_point[1].eval_step = 8_000;
    assert!(log.validate().is_err());
}

#[test]
fn loss_term_eval_point_rejects_mismatched_raw_weighted_nullability() {
    let mut terms = loss_terms(4_000, true);
    terms
        .weighted_losses
        .insert("distill".to_owned(), Some(0.0));

    assert!(terms.validate().is_err());
}

fn ternary_score() -> Result<S2ScoreReport, gbf_experiments::s1::schema::S1SchemaError> {
    S2ScoreReport::new(
        0,
        S2BuildKind::s2_ternary_full,
        hash(1),
        hash(2),
        4_096,
        8_192.0,
        Some(threshold_stats()),
        Some(scale_stats()),
    )
}

fn nodistill_log() -> Result<DistillationLog, gbf_experiments::s1::schema::S1SchemaError> {
    DistillationLog::new(
        0,
        S2BuildKind::s2_ternary_nodistill,
        hash(3),
        hash(4),
        hash(5),
        2.0,
        0.0,
        vec![
            DistillEvalPoint {
                eval_step: 4_000,
                distill_loss: None,
            },
            DistillEvalPoint {
                eval_step: 5_001,
                distill_loss: Some(0.25),
            },
        ],
        hash(6),
        vec![loss_terms(4_000, true), loss_terms(5_001, false)],
    )
}

fn loss_terms(eval_step: u64, pre_phase_c: bool) -> LossTermEvalPoint {
    let mut raw_losses = BTreeMap::new();
    let mut weighted_losses = BTreeMap::new();
    raw_losses.insert("distill".to_owned(), (!pre_phase_c).then_some(0.25));
    weighted_losses.insert("distill".to_owned(), (!pre_phase_c).then_some(0.0));
    raw_losses.insert("router_balance".to_owned(), None);
    weighted_losses.insert("router_balance".to_owned(), None);
    LossTermEvalPoint {
        eval_step,
        lambda_effective: PhaseEffectiveLambda::new(PhaseEffectiveLambdaValues {
            lambda_distill: 0.0,
            lambda_balance: 0.0,
            lambda_zrouter: 0.0,
            lambda_switch: 0.0,
            lambda_range: 0.0,
            lambda_zero: if pre_phase_c { 0.0 } else { 0.0001 },
            lambda_shape: 0.0,
            lambda_overflow: 0.0,
        })
        .expect("lambdas"),
        raw_losses,
        weighted_losses,
    }
}

fn threshold_stats() -> ThresholdStatsSummary {
    ThresholdStatsSummary {
        matrices: 2,
        threshold_min: 0.1,
        threshold_max: 0.4,
        threshold_mean: 0.25,
        threshold_count: 8,
    }
}

fn scale_stats() -> ScaleStatsSummary {
    ScaleStatsSummary {
        matrices: 2,
        scale_count: 8,
        scale_min: 0.5,
        scale_max: 1.5,
        scale_mean_f32: 1.0,
    }
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
