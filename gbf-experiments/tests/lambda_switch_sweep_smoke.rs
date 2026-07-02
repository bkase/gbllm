#![cfg(feature = "s7-router-collapse-sweep")]

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s7::collapse_sweep::{
    CollapseSweepError, D11_LAMBDA_SWITCH_GRID, D11_LAMBDA_SWITCH_SWEEP_SEED,
    D11_PRODUCTION_LAMBDA_SWITCH, DETERMINISTIC_FIXTURE_SWEEP_PRODUCER_KIND,
    DeterministicFixtureSweepProducer, GuardrailVerdict, LAMBDA_SWITCH_SWEEP_STEP_EVENT,
    LambdaSwitchSweepCompletion, LambdaSwitchSweepInput, LambdaSwitchSweepPointInput,
    LambdaSwitchSweepPointOutcome, LambdaSwitchSweepProducer, LambdaSwitchSweepRecord,
    PRODUCTION_SWEEP_PRODUCER_KIND, RCS_TRAINING_EXTRA_STEPS, ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA,
    RouterCollapseSweepReport, canonicalize_d11_lambda_switch_grid,
    f8_constant_lambda_sweep_verdict, lambda_switch_grid_hash, run_lambda_switch_sweep,
};
use gbf_foundation::Hash256;
use serde_json::json;

const SEED: u64 = 0;
const BASE_TRAIN_STEP: u64 = 16_000;
const VAL_EVAL_SUBSET_LEN: u64 = 4_096;

#[test]
fn d11_fixture_sweep_emits_exact_six_deterministic_records() {
    let input = sweep_input(0x6d);
    let first = run_lambda_switch_sweep(&input, &DeterministicFixtureSweepProducer)
        .expect("fixture sweep report");
    let second = run_lambda_switch_sweep(&input, &DeterministicFixtureSweepProducer)
        .expect("fixture sweep report replays");

    assert_eq!(first, second);
    assert_eq!(first.schema, ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA);
    assert_eq!(
        first.producer_kind,
        DETERMINISTIC_FIXTURE_SWEEP_PRODUCER_KIND
    );
    assert_eq!(first.guardrail_verdict, GuardrailVerdict::Pass);
    assert_eq!(first.records.len(), D11_LAMBDA_SWITCH_GRID.len());
    assert_ne!(first.sweep_self_hash, Hash256::ZERO);
    assert_eq!(
        first
            .grid
            .iter()
            .map(|lambda| lambda.to_bits())
            .collect::<Vec<_>>(),
        D11_LAMBDA_SWITCH_GRID
            .iter()
            .map(|lambda| lambda.to_bits())
            .collect::<Vec<_>>()
    );

    for (record, lambda_switch) in first.records.iter().zip(D11_LAMBDA_SWITCH_GRID) {
        assert_eq!(record.seed, SEED);
        assert_eq!(record.lambda_switch.to_bits(), lambda_switch.to_bits());
        assert_eq!(record.base_train_step, BASE_TRAIN_STEP);
        assert_eq!(
            record.training_extra_step_delta().unwrap(),
            RCS_TRAINING_EXTRA_STEPS
        );
        assert!(record.bpc_eval_subset.is_some());
        assert!(record.expert_usage_entropy_bits_mean.is_finite());
        assert!(record.expert_usage_entropy_bits_mean <= 2.0);
        assert_ne!(record.sweep_self_hash, Hash256::ZERO);
        record
            .verify_self_hash()
            .expect("record self-hash verifies");
    }

    assert_eq!(
        first.canonical_json_bytes().unwrap(),
        second.canonical_json_bytes().unwrap()
    );
    let changed_base = sweep_input(0x7e);
    let changed = run_lambda_switch_sweep(&changed_base, &DeterministicFixtureSweepProducer)
        .expect("changed-base fixture sweep report");
    assert_ne!(first.sweep_self_hash, changed.sweep_self_hash);
}

#[test]
fn router_collapse_sweep_json_shape_pins_downstream_fields() {
    let input = sweep_input(0x51);
    let report = run_lambda_switch_sweep(&input, &DeterministicFixtureSweepProducer)
        .expect("fixture sweep report");
    let value = serde_json::to_value(&report).expect("report serializes");

    assert_eq!(value["schema"], json!(ROUTER_COLLAPSE_SWEEP_REPORT_SCHEMA));
    assert_eq!(value["seed"], json!(SEED));
    assert_eq!(value["base_checkpoint_sha"], json!(hash(0x51)));
    assert_eq!(
        value["producer_kind"],
        json!(DETERMINISTIC_FIXTURE_SWEEP_PRODUCER_KIND)
    );
    assert_eq!(value["grid"], json!(D11_LAMBDA_SWITCH_GRID));
    assert_eq!(
        value["production_lambda"],
        json!(D11_PRODUCTION_LAMBDA_SWITCH)
    );
    assert_eq!(value["records"].as_array().unwrap().len(), 6);
    assert!(value["sweep_self_hash"].is_string());

    let production = value["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["lambda_switch"] == json!(D11_PRODUCTION_LAMBDA_SWITCH))
        .expect("production-lambda record");
    assert_eq!(production["seed"], json!(SEED));
    assert_eq!(production["base_train_step"], json!(BASE_TRAIN_STEP));
    assert_eq!(
        production["train_step"],
        json!(BASE_TRAIN_STEP + RCS_TRAINING_EXTRA_STEPS)
    );
    assert!(production["expert_usage_entropy_bits_mean"].is_number());
    assert!(production["quality_delta_per_lambda_switch"].is_number());
    assert!(production["sweep_self_hash"].is_string());
}

#[test]
fn producer_input_carries_validated_run_descriptor_contract() {
    let input = sweep_input(0x41);
    let report =
        run_lambda_switch_sweep(&input, &ContractAssertingProducer).expect("contract sweep report");

    assert_eq!(report.base_checkpoint_sha, input.base_checkpoint_sha);
    assert_eq!(report.producer_kind, PRODUCTION_SWEEP_PRODUCER_KIND);
    assert_eq!(report.records.len(), D11_LAMBDA_SWITCH_GRID.len());
    assert_eq!(report.guardrail_verdict, GuardrailVerdict::Pass);
}

#[test]
fn sweep_descriptor_rejects_non_seed_zero_and_bad_point_contract() {
    let err = LambdaSwitchSweepInput::d11(
        1,
        hash(0x41),
        BASE_TRAIN_STEP,
        hash(0xa7),
        VAL_EVAL_SUBSET_LEN,
    )
    .expect_err("D11 sweep is pinned to seed 0");
    assert!(matches!(
        err,
        CollapseSweepError::UnexpectedSweepSeed {
            observed: 1,
            expected: D11_LAMBDA_SWITCH_SWEEP_SEED,
        }
    ));

    let point = LambdaSwitchSweepPointInput {
        seed: SEED,
        base_checkpoint_sha: hash(0x41),
        base_train_step: BASE_TRAIN_STEP,
        val_eval_subset_sha: hash(0xa7),
        val_eval_subset_len: VAL_EVAL_SUBSET_LEN,
        extra_train_steps: RCS_TRAINING_EXTRA_STEPS - 1,
        lambda_switch: D11_PRODUCTION_LAMBDA_SWITCH,
        lambda_switch_grid_hash: lambda_switch_grid_hash(&D11_LAMBDA_SWITCH_GRID)
            .expect("grid hash"),
    };
    assert!(matches!(
        point.validate(),
        Err(CollapseSweepError::UnexpectedSweepExtraTrainSteps {
            observed: 999,
            expected: RCS_TRAINING_EXTRA_STEPS,
        })
    ));
}

#[test]
fn sweep_step_event_shape_is_subscriber_captured() {
    let input = sweep_input(0x44);
    let capture = TraceCapture::default();
    let report = with_trace_capture(&capture, || {
        run_lambda_switch_sweep(&input, &DeterministicFixtureSweepProducer)
            .expect("fixture sweep report")
    });
    let events = captured_events(&capture);
    let sweep_events = events
        .iter()
        .filter(|event| event.name == LAMBDA_SWITCH_SWEEP_STEP_EVENT)
        .collect::<Vec<_>>();

    assert_eq!(report.records.len(), D11_LAMBDA_SWITCH_GRID.len());
    assert_eq!(sweep_events.len(), D11_LAMBDA_SWITCH_GRID.len());
    let production = sweep_events[1];
    assert!(production.fields.contains_key("lambda_switch"));
    assert_eq!(
        production.fields.get("event_name"),
        Some(&json!(LAMBDA_SWITCH_SWEEP_STEP_EVENT))
    );
    assert_eq!(production.fields.get("seed"), Some(&json!(SEED)));
    assert_eq!(
        production.fields.get("base_train_step"),
        Some(&json!(BASE_TRAIN_STEP))
    );
    assert_eq!(
        production.fields.get("extra_train_steps"),
        Some(&json!(RCS_TRAINING_EXTRA_STEPS))
    );
    assert!(production.fields.contains_key("bpc_eval_subset"));
    assert!(
        production
            .fields
            .contains_key("expert_usage_entropy_bits_mean")
    );
    assert!(
        production
            .fields
            .contains_key("quality_delta_per_lambda_switch")
    );
    assert!(production.fields.contains_key("sweep_self_hash"));
    assert!(production.fields.contains_key("sweep_canonical_json"));
}

#[test]
fn non_high_lambda_divergence_is_reported_as_inconclusive() {
    let input = sweep_input(0x22);
    let report = run_lambda_switch_sweep(&input, &NonHighDivergenceProducer)
        .expect("divergent fixture sweep report");

    assert_eq!(
        report.guardrail_verdict,
        GuardrailVerdict::InconclusiveDiverged {
            lambda_switch: 0.5,
            step: BASE_TRAIN_STEP + 250,
        }
    );
    let diverged = report
        .records
        .iter()
        .find(|record| record.lambda_switch.to_bits() == 0.5_f32.to_bits())
        .expect("0.5 record");
    assert_eq!(
        diverged.completion,
        LambdaSwitchSweepCompletion::DivergedAt {
            step: BASE_TRAIN_STEP + 250
        }
    );
    assert_eq!(diverged.bpc_eval_subset, None);
    assert_eq!(diverged.quality_delta_per_lambda_switch, None);
}

#[test]
fn f8_constant_lambda_degenerate_sweep_is_fail_c_not_valid_d11_report() {
    let production = LambdaSwitchSweepRecord::successful(
        D11_PRODUCTION_LAMBDA_SWITCH,
        BASE_TRAIN_STEP,
        1.02,
        1.86,
        1.02,
    )
    .expect("production record");

    assert_eq!(
        f8_constant_lambda_sweep_verdict(&[production]).unwrap(),
        GuardrailVerdict::FailC
    );
    let err = RouterCollapseSweepReport::from_grid_records(
        SEED,
        hash(0x33),
        vec![D11_PRODUCTION_LAMBDA_SWITCH],
        vec![production],
    )
    .expect_err("constant-lambda grid is not a valid D11 report");
    assert!(matches!(
        err,
        CollapseSweepError::UnexpectedGridCount { .. }
    ));
}

#[test]
fn d11_grid_hash_and_membership_use_exact_bit_patterns() {
    let canonical =
        canonicalize_d11_lambda_switch_grid(&D11_LAMBDA_SWITCH_GRID).expect("canonical grid");
    assert_eq!(
        lambda_switch_grid_hash(&canonical).unwrap(),
        lambda_switch_grid_hash(&D11_LAMBDA_SWITCH_GRID).unwrap()
    );

    let mut bad_grid = D11_LAMBDA_SWITCH_GRID.to_vec();
    bad_grid[1] = f32::from_bits(D11_LAMBDA_SWITCH_GRID[1].to_bits() + 1);
    let err =
        canonicalize_d11_lambda_switch_grid(&bad_grid).expect_err("bit-drifted grid must fail");
    assert!(matches!(
        err,
        CollapseSweepError::UnexpectedGridValue { index: 1, .. }
    ));
}

struct ContractAssertingProducer;

impl LambdaSwitchSweepProducer for ContractAssertingProducer {
    fn run_sweep_point(
        &self,
        input: LambdaSwitchSweepPointInput,
    ) -> Result<LambdaSwitchSweepPointOutcome, CollapseSweepError> {
        assert_eq!(input.seed, SEED);
        assert_eq!(input.base_checkpoint_sha, hash(0x41));
        assert_eq!(input.base_train_step, BASE_TRAIN_STEP);
        assert_eq!(input.val_eval_subset_sha, hash(0xa7));
        assert_eq!(input.val_eval_subset_len, VAL_EVAL_SUBSET_LEN);
        assert_eq!(input.extra_train_steps, RCS_TRAINING_EXTRA_STEPS);
        DeterministicFixtureSweepProducer.run_sweep_point(input)
    }
}

struct NonHighDivergenceProducer;

impl LambdaSwitchSweepProducer for NonHighDivergenceProducer {
    fn run_sweep_point(
        &self,
        input: LambdaSwitchSweepPointInput,
    ) -> Result<LambdaSwitchSweepPointOutcome, CollapseSweepError> {
        if input.lambda_switch.to_bits() == 0.5_f32.to_bits() {
            return LambdaSwitchSweepPointOutcome::diverged_at(BASE_TRAIN_STEP + 250, 1.70);
        }
        DeterministicFixtureSweepProducer.run_sweep_point(input)
    }
}

fn sweep_input(base_checkpoint_byte: u8) -> LambdaSwitchSweepInput {
    LambdaSwitchSweepInput::d11(
        SEED,
        hash(base_checkpoint_byte),
        BASE_TRAIN_STEP,
        hash(0xa7),
        VAL_EVAL_SUBSET_LEN,
    )
    .expect("valid D11 sweep input")
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
