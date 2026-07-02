#![cfg(feature = "s7")]

use gbf_artifact::{S7ScoreReport, S7Topology};
use gbf_experiments::s7::collapse_sweep::{
    CollapseSweepError, D11_LAMBDA_SWITCH_GRID, D11_PRODUCTION_LAMBDA_SWITCH, GuardrailVerdict,
    H3_PARITY_MARGIN_BPC, H6_HIGH_LAMBDA_SWITCH, LambdaSwitchSweepCompletion,
    LambdaSwitchSweepRecord, RCS_TRAINING_EXTRA_STEPS, S7ParitySeedVerdict, h6_guardrail_verdict,
    s7_parity_seed_from_production_scores, validate_collapse_sweep_records,
};
use gbf_foundation::Hash256;

const RFC: &str = include_str!("../../history/rfcs/F-S7-moe-beats-dense.md");
const BASE_TRAIN_STEP: u64 = 16_000;

#[test]
fn collapse_sweep_requires_one_1000_extra_step_record_per_grid_point() {
    let base_train_step = 16_000;
    let records = D11_LAMBDA_SWITCH_GRID
        .iter()
        .copied()
        .map(|lambda_switch| {
            LambdaSwitchSweepRecord::successful(
                lambda_switch,
                base_train_step,
                1.0 + f64::from(lambda_switch),
                1.8,
                1.05,
            )
            .expect("valid sweep record")
        })
        .collect::<Vec<_>>();

    validate_collapse_sweep_records(&records).expect("valid sweep");
    for record in records {
        assert_eq!(
            record.training_extra_step_delta().unwrap(),
            RCS_TRAINING_EXTRA_STEPS
        );
    }
}

#[test]
fn collapse_sweep_rejects_wrong_extra_step_delta() {
    let err = LambdaSwitchSweepRecord::from_parts(
        D11_PRODUCTION_LAMBDA_SWITCH,
        16_000,
        16_999,
        Some(1.0),
        1.8,
        Some(0.0),
    )
    .expect_err("999-step sweep record must fail");

    assert!(matches!(
        err,
        CollapseSweepError::UnexpectedTrainingExtraStepDelta {
            observed: 999,
            expected: RCS_TRAINING_EXTRA_STEPS,
            ..
        }
    ));
}

#[test]
fn collapse_sweep_allows_only_null_bpc_for_divergent_records() {
    let err = LambdaSwitchSweepRecord::from_parts_with_completion(
        H6_HIGH_LAMBDA_SWITCH,
        BASE_TRAIN_STEP,
        BASE_TRAIN_STEP + RCS_TRAINING_EXTRA_STEPS,
        LambdaSwitchSweepCompletion::DivergedAt {
            step: BASE_TRAIN_STEP + 250,
        },
        Some(9.99),
        0.8,
        None,
    )
    .expect_err("divergent sweep records must carry null bpc");

    assert!(matches!(
        err,
        CollapseSweepError::DivergedRecordHasBpc { .. }
    ));
}

#[test]
fn h6_guardrail_checks_a_b_c_or_d_independently() {
    let pass = h6_records(
        1.02,
        1.80,
        HighPoint::Completed {
            bpc: 1.40,
            entropy_bits: 1.30,
        },
    );
    assert_eq!(h6_guardrail_verdict(&pass).unwrap(), GuardrailVerdict::Pass);

    let fail_a = h6_records(
        1.06,
        1.80,
        HighPoint::Completed {
            bpc: 1.40,
            entropy_bits: 1.30,
        },
    );
    assert_eq!(
        h6_guardrail_verdict(&fail_a).unwrap(),
        GuardrailVerdict::FailA
    );

    let fail_b = h6_records(
        1.02,
        1.69,
        HighPoint::Completed {
            bpc: 1.40,
            entropy_bits: 1.30,
        },
    );
    assert_eq!(
        h6_guardrail_verdict(&fail_b).unwrap(),
        GuardrailVerdict::FailB
    );

    let fail_c = h6_records(
        1.02,
        1.80,
        HighPoint::Completed {
            bpc: 1.40,
            entropy_bits: 1.55,
        },
    );
    assert_eq!(
        h6_guardrail_verdict(&fail_c).unwrap(),
        GuardrailVerdict::FailC
    );

    let fail_d = h6_records(
        1.02,
        1.80,
        HighPoint::Completed {
            bpc: 1.25,
            entropy_bits: 1.30,
        },
    );
    assert_eq!(
        h6_guardrail_verdict(&fail_d).unwrap(),
        GuardrailVerdict::FailD
    );
}

#[test]
fn h6_guardrail_returns_inconclusive_for_unrecovered_high_lambda_divergence() {
    let records = h6_records(
        1.02,
        1.80,
        HighPoint::Diverged {
            step: BASE_TRAIN_STEP + 500,
            last_finite_entropy_bits: 1.65,
        },
    );

    assert_eq!(
        h6_guardrail_verdict(&records).unwrap(),
        GuardrailVerdict::InconclusiveDiverged {
            lambda_switch: H6_HIGH_LAMBDA_SWITCH,
            step: BASE_TRAIN_STEP + 500,
        }
    );
}

#[test]
fn h6_guardrail_returns_inconclusive_for_non_high_lambda_divergence() {
    let divergent_lambda = 0.5;
    let divergence_step = BASE_TRAIN_STEP + 250;
    let mut records = h6_records(
        1.02,
        1.80,
        HighPoint::Completed {
            bpc: 1.40,
            entropy_bits: 1.30,
        },
    );
    let record = records
        .iter_mut()
        .find(|record| lambda_is(record.lambda_switch, divergent_lambda))
        .expect("non-high-lambda grid record");
    *record =
        LambdaSwitchSweepRecord::diverged(divergent_lambda, BASE_TRAIN_STEP, divergence_step, 1.70)
            .expect("non-high divergent record");

    assert_eq!(
        h6_guardrail_verdict(&records).unwrap(),
        GuardrailVerdict::InconclusiveDiverged {
            lambda_switch: divergent_lambda,
            step: divergence_step,
        }
    );
}

#[test]
fn h6_guardrail_recovers_high_lambda_divergence_when_entropy_collapse_is_proven() {
    let records = h6_records(
        1.02,
        1.80,
        HighPoint::Diverged {
            step: BASE_TRAIN_STEP + 500,
            last_finite_entropy_bits: 1.20,
        },
    );

    assert_eq!(
        h6_guardrail_verdict(&records).unwrap(),
        GuardrailVerdict::Pass
    );

    let high = records
        .iter()
        .find(|record| record.lambda_switch.to_bits() == H6_HIGH_LAMBDA_SWITCH.to_bits())
        .expect("high-lambda record");
    assert_eq!(
        high.completion,
        LambdaSwitchSweepCompletion::DivergedAt {
            step: BASE_TRAIN_STEP + 500,
        }
    );
    assert_eq!(high.bpc_eval_subset, None);
    assert_eq!(high.quality_delta_per_lambda_switch, None);
}

#[test]
fn h3_uses_production_score_reports_not_sweep_local_bpc() {
    let moe_score = score(S7Topology::MoeTiny, 0, 1.00);
    let dense_score = score(S7Topology::MoeTinyDenseMatched, 0, 1.10);
    let sweep_local_production =
        LambdaSwitchSweepRecord::successful(D11_PRODUCTION_LAMBDA_SWITCH, 16_000, 9.99, 1.8, 9.99)
            .expect("valid sweep-local production record");

    let verdict =
        s7_parity_seed_from_production_scores(&moe_score, &dense_score).expect("h3 verdict");

    assert_eq!(verdict, S7ParitySeedVerdict::Pass);
    assert_ne!(
        sweep_local_production.bpc_eval_subset,
        Some(moe_score.bpc),
        "sweep-local bpc is intentionally not the H3 input"
    );
    assert_eq!(H3_PARITY_MARGIN_BPC, 0.05);
}

#[test]
fn h3_rejects_non_production_score_pair_shape() {
    let moe_seed_0 = score(S7Topology::MoeTiny, 0, 1.00);
    let dense_seed_1 = score(S7Topology::MoeTinyDenseMatched, 1, 1.10);

    let err = s7_parity_seed_from_production_scores(&moe_seed_0, &dense_seed_1)
        .expect_err("seed mismatch must fail");

    assert!(matches!(err, CollapseSweepError::ScoreSeedMismatch { .. }));
}

#[test]
fn rfc_pins_d19_h3_and_rcs_cadence_language() {
    let d19 = rfc_section(
        "D19 Standard producer telemetry, every step",
        "# 1. Hypothesis algebra",
    );
    assert!(!d19.contains("every 1000 steps"));
    assert!(d19.contains("In the post-Phase-D sweep harness"));
    assert!(d19.contains("quality_delta_per_lambda_switch"));

    let h5 = rfc_section(
        "## H5 Router switch-awareness",
        "## H6 Router collapse guardrail",
    );
    assert!(h5.contains("exactly one 1000-extra-step"));
    assert!(h5.contains("record per grid point"));

    let d11 = rfc_section("D11 lambda_switch sweep", "D12 Matched-bytes parity");
    assert!(d11.contains("used only for H6 guardrail comparisons"));
    assert!(d11.contains("H3 uses the final production training run's validation bpc"));

    let parity = rfc_section(
        "## 11.1 Per-seed parity check",
        "## 11.2 Aggregate parity verdict",
    );
    assert!(parity.contains("production_moe_score: S7ScoreReport"));
    assert!(parity.contains("production_dense_matched_score: S7ScoreReport"));
    assert!(parity.contains("not sweep-local records"));

    let sweep = rfc_section(
        "## 9.3 lambda_switch sweep telemetry",
        "# 10. Router collapse guardrail contract",
    );
    assert!(sweep.contains("trained checkpoint (seed=0, end of Phase D)"));
    assert!(sweep.contains("re-train for 1000 additional steps"));

    let report = rfc_section(
        "## 13.5 s7_router_collapse_sweep.v1",
        "## 13.6 s7_dense_vs_moe.v1",
    );
    assert!(report.contains("RCS-Cadence"));
    assert!(report.contains("training-extra step delta = 1000"));
    assert!(report.contains("RCS-Diverged"));
}

#[test]
fn rfc_pins_h6_divergent_guardrail_language() {
    let h6 = rfc_section(
        "## H6 Router collapse guardrail",
        "## H7 Loss gradient provenance",
    );
    assert!(h6.contains("failure of A, B, C, or D"));
    assert!(!h6.contains("A, B, or C+D"));
    assert!(!h6.contains("A, B, or (C and D)"));
    assert!(!h6.contains("A, B, or C and D"));

    let schema = rfc_section("LambdaSwitchSweepStep :=", "## 3.7 RouterRng");
    assert!(schema.contains("completion:                     Completed | DivergedAt(TrainStep)"));
    assert!(schema.contains("bpc_eval_subset:                Null | BpcValue"));
    assert!(schema.contains("LSS-Diverged"));

    let sweep = rfc_section(
        "## 9.3 lambda_switch sweep telemetry",
        "# 10. Router collapse guardrail contract",
    );
    assert!(sweep.contains("record completion = DivergedAt(step)"));
    assert!(sweep.contains("bpc_eval_subset = null"));
    assert!(sweep.contains("expert_usage_entropy_bits_mean = last finite observed value"));
    assert!(sweep.contains("GuardrailVerdict = InconclusiveDiverged unless"));

    let guardrail = rfc_section(
        "# 10. Router collapse guardrail contract",
        "# 11. Matched-bytes parity gate",
    );
    assert!(guardrail.contains("InconclusiveDiverged(lambda_switch, step)"));
    assert!(guardrail.contains("elif bpc_production - bpc_baseline > 0.05"));
    assert!(guardrail.contains("elif ent_production < 0.85 * log2_n_experts"));
    assert!(guardrail.contains("elif (ent_production - ent_high) < 0.3"));
    assert!(guardrail.contains("elif (bpc_high - bpc_production) < 0.3"));
    assert!(guardrail.contains(
        "InconclusiveDiverged   ⇒ H6 Refuted; S7Outcome = Fail-router-collapse-guardrail"
    ));
}

#[derive(Clone, Copy)]
enum HighPoint {
    Completed {
        bpc: f64,
        entropy_bits: f32,
    },
    Diverged {
        step: u64,
        last_finite_entropy_bits: f32,
    },
}

fn h6_records(
    production_bpc: f64,
    production_entropy_bits: f32,
    high: HighPoint,
) -> Vec<LambdaSwitchSweepRecord> {
    D11_LAMBDA_SWITCH_GRID
        .into_iter()
        .map(|lambda_switch| {
            if lambda_is(lambda_switch, 0.0) {
                return LambdaSwitchSweepRecord::successful(
                    lambda_switch,
                    BASE_TRAIN_STEP,
                    1.00,
                    1.90,
                    production_bpc,
                )
                .expect("baseline sweep record");
            }
            if lambda_is(lambda_switch, D11_PRODUCTION_LAMBDA_SWITCH) {
                return LambdaSwitchSweepRecord::successful(
                    lambda_switch,
                    BASE_TRAIN_STEP,
                    production_bpc,
                    production_entropy_bits,
                    production_bpc,
                )
                .expect("production sweep record");
            }
            if lambda_is(lambda_switch, H6_HIGH_LAMBDA_SWITCH) {
                return match high {
                    HighPoint::Completed { bpc, entropy_bits } => {
                        LambdaSwitchSweepRecord::successful(
                            lambda_switch,
                            BASE_TRAIN_STEP,
                            bpc,
                            entropy_bits,
                            production_bpc,
                        )
                        .expect("high-lambda sweep record")
                    }
                    HighPoint::Diverged {
                        step,
                        last_finite_entropy_bits,
                    } => LambdaSwitchSweepRecord::diverged(
                        lambda_switch,
                        BASE_TRAIN_STEP,
                        step,
                        last_finite_entropy_bits,
                    )
                    .expect("divergent high-lambda sweep record"),
                };
            }

            LambdaSwitchSweepRecord::successful(
                lambda_switch,
                BASE_TRAIN_STEP,
                1.05 + f64::from(lambda_switch),
                1.80,
                production_bpc,
            )
            .expect("non-gating sweep record")
        })
        .collect()
}

fn score(topology: S7Topology, seed: u64, bpc: f64) -> S7ScoreReport {
    let token_count = 1_000;
    S7ScoreReport::new(
        seed,
        topology,
        Hash256::ZERO,
        Hash256::ZERO,
        token_count,
        bpc * token_count as f64,
    )
    .expect("score")
    .with_computed_self_hash()
    .expect("score self hash")
}

fn lambda_is(lambda_switch: f32, target: f32) -> bool {
    lambda_switch.to_bits() == target.to_bits()
}

fn rfc_section(start: &str, end: &str) -> &'static str {
    let start_index = RFC.find(start).expect("section start");
    let rest = &RFC[start_index..];
    let end_index = rest.find(end).expect("section end");
    &rest[..end_index]
}
