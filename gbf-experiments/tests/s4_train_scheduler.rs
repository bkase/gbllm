#![cfg(feature = "s4")]

use std::collections::BTreeMap;

mod common;

use common::tracing_capture::{TraceCapture, TracingEvent, captured_events, with_trace_capture};
use gbf_experiments::s4::rng::{
    S4_BATCH_RNG_DOMAIN, S4_INIT_RNG_DOMAIN, S4_SHUFFLE_RNG_DOMAIN, S4RngStreams, s4_seed128,
};
use gbf_experiments::s4::run::{
    S4_BATCHRNG_INSTANTIATED_EVENT_NAME, S4_CONTINUATION_INIT_STARTED_EVENT_NAME,
    S4_OPTIMIZER_STATE_ZEROED_EVENT_NAME, S4_RESUMED_PHASE_D_EVENT_NAME, S4_STEP_EVENT_NAME,
    S4ContinuationInitInputs, S4PhaseDContinuationSchedule, S4RunScheduleError, S4SeedRunLoop,
    canonical_seed_run_contracts, initialize_gutenberg_continuation, progress_eval_steps,
    required_train_loss_count, seed_run_contracts_for_ordered_seeds,
};
use gbf_experiments::s4::schema::{
    S4_BATCH_SIZE, S4_CANONICAL_SEEDS, S4_D10_ESTIMATE_TAG, S4_EVAL_EVERY_STEPS,
    S4_EVAL_SUBSET_SIZE, S4_GUTENBERG_ADAMW_LR, S4_OPTIMIZER_STEPS_GUTENBERG, S4_SEQUENCE_LENGTH,
    S4AdamWConfig, S4DeviceProfileKind, S4Hypothesis, S4InitialWeightSource,
    S4OptimizerStateInitial, S4QatShadowWeightSource, S4RngKind, S4SchemaError,
    S4TernaryProjectionInitial, S4TrainConfig, S4TrainPhase, S4VerifierBundle, train_config_hash,
    validate_s4_canonical_seed_list,
};
use gbf_foundation::Hash256;
use gbf_model::qat::RouterTrainMode;
use gbf_train::phase::{QuantHardness, TrainPhaseKind};
use serde_json::json;

#[test]
fn s4_train_config_pins_d9_d10_gutenberg_contract() {
    let config = S4TrainConfig::pinned();

    assert_eq!(config.optimizer_steps, S4_OPTIMIZER_STEPS_GUTENBERG);
    assert_eq!(config.optimizer_steps, 20_000);
    assert_eq!(config.batch_size, S4_BATCH_SIZE);
    assert_eq!(config.batch_size, 32);
    assert_eq!(config.sequence_length, S4_SEQUENCE_LENGTH);
    assert_eq!(config.sequence_length, 128);
    assert_eq!(config.eval_every_steps, S4_EVAL_EVERY_STEPS);
    assert_eq!(config.eval_every_steps, 2_000);
    assert_eq!(config.eval_subset_size, S4_EVAL_SUBSET_SIZE);
    assert_eq!(config.eval_subset_size, 4_096);
    assert_eq!(config.optimizer, S4AdamWConfig::gutenberg_d10());
    assert_eq!(config.optimizer.lr, S4_GUTENBERG_ADAMW_LR);
    assert_eq!(config.phase, S4TrainPhase::PhaseD);
    assert_eq!(config.rng_kind, S4RngKind::Pcg64Mcg);
    assert_eq!(
        config.device_profile,
        S4DeviceProfileKind::S1CpuDeterministic
    );
    config.validate().expect("pinned S4 config validates");

    let json = serde_json::to_value(&config).expect("config serializes");
    assert_eq!(json["optimizer_steps"], json!(20_000));
    assert_eq!(json["phase"], json!("phase_d"));
    assert_eq!(json["rng_kind"], json!("pcg64_mcg"));
    assert_eq!(json["device_profile"], json!("s1_cpu_deterministic"));
    assert!((json["optimizer"]["lr"].as_f64().expect("lr is numeric") - 0.0005).abs() < 1.0e-9);
}

#[test]
fn s4_train_config_hash_is_deterministic_and_rejects_d10_drift() {
    let config = S4TrainConfig::pinned();
    let hash_a = train_config_hash(&config).expect("config hashes");
    let hash_b = train_config_hash(&config).expect("config hashes deterministically");
    assert_eq!(hash_a, hash_b);

    for (field, mutated) in mutated_configs(&config) {
        let error = train_config_hash(&mutated).expect_err("mutated D10 config is rejected");
        assert!(
            matches!(
                error,
                S4SchemaError::NonCanonicalTrainConfig { field: observed }
                    if observed == field
            ),
            "{field} should be rejected as non-canonical, got {error:?}"
        );
    }
}

#[test]
fn train_config_built_log_preserves_d10_estimate_tag() {
    let config = S4TrainConfig::pinned();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        train_config_hash(&config).expect("config hashes");
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "train_config_built")
        .expect("train_config_built event");
    assert_eq!(
        event.fields.get("d10_optimizer_steps_tag"),
        Some(&json!(S4_D10_ESTIMATE_TAG))
    );
    assert_eq!(
        event.fields.get("optimizer_steps_gutenberg"),
        Some(&json!(20_000))
    );
}

#[test]
fn phase_d_continuation_descriptor_uses_fully_hardened_gbf_train_controls() {
    let schedule = S4PhaseDContinuationSchedule::pinned().expect("pinned schedule");
    let phase = schedule.phase();

    assert_eq!(phase.kind(), TrainPhaseKind::FullNumericQat);
    assert_eq!(phase.start_step(), 0);
    assert_eq!(phase.end_step(), S4_OPTIMIZER_STEPS_GUTENBERG);
    assert_eq!(phase.len_steps(), S4_OPTIMIZER_STEPS_GUTENBERG);
    assert_eq!(phase.expert_qat(), QuantHardness::Hard);
    assert_eq!(phase.activation_qat(), QuantHardness::Hard);
    assert_eq!(phase.norm_qat(), QuantHardness::Hard);
    assert_eq!(phase.router_mode(), RouterTrainMode::HardTop1);

    assert_eq!(schedule.scheduler_step_for_optimizer_step(1).unwrap(), 0);
    assert_eq!(
        schedule
            .scheduler_step_for_optimizer_step(S4_OPTIMIZER_STEPS_GUTENBERG)
            .unwrap(),
        S4_OPTIMIZER_STEPS_GUTENBERG - 1
    );
    assert_eq!(schedule.phase_for_optimizer_step(1).unwrap(), phase);
    assert_eq!(
        schedule
            .phase_for_optimizer_step(S4_OPTIMIZER_STEPS_GUTENBERG)
            .unwrap(),
        phase
    );
    assert_eq!(
        schedule.scheduler_step_for_optimizer_step(0).unwrap_err(),
        S4RunScheduleError::StepOutOfRange {
            step: 0,
            max_step: S4_OPTIMIZER_STEPS_GUTENBERG
        }
    );
    assert_eq!(
        schedule
            .scheduler_step_for_optimizer_step(S4_OPTIMIZER_STEPS_GUTENBERG + 1)
            .unwrap_err(),
        S4RunScheduleError::StepOutOfRange {
            step: S4_OPTIMIZER_STEPS_GUTENBERG + 1,
            max_step: S4_OPTIMIZER_STEPS_GUTENBERG
        }
    );
}

#[test]
fn dynamic_run_loop_reaches_exact_d10_budget_and_refuses_step_20001() {
    let config = S4TrainConfig::pinned();
    let mut run_loop = S4SeedRunLoop::new(0, &config).expect("seed run loop");

    let evidence = run_loop.run_to_budget().expect("run reaches D10 budget");

    assert_eq!(evidence.seed, 0);
    assert_eq!(evidence.completed_optimizer_steps, 20_000);
    assert_eq!(evidence.final_optimizer_step, Some(20_000));
    assert_eq!(evidence.event_history.len(), 20_000);
    assert_eq!(run_loop.completed_optimizer_steps(), 20_000);

    let first = evidence.event_history.first().expect("first step");
    assert_eq!(first.step, 1);
    assert_eq!(first.scheduler_step, 0);
    let last = evidence.event_history.last().expect("last step");
    assert_eq!(last.step, S4_OPTIMIZER_STEPS_GUTENBERG);
    assert_eq!(last.scheduler_step, S4_OPTIMIZER_STEPS_GUTENBERG - 1);

    for (idx, event) in evidence.event_history.iter().enumerate() {
        assert_eq!(event.seed, 0);
        assert_eq!(event.step, idx as u64 + 1);
        assert_eq!(event.scheduler_step, idx as u64);
        assert_eq!(event.phase, S4TrainPhase::PhaseD);
        assert_eq!(event.phase_label(), "D");
    }

    let error = run_loop
        .advance_one_step()
        .expect_err("step 20001 is refused");
    assert_eq!(error.name(), "S4OptimizerStepBudgetExceeded");
    assert_eq!(
        error,
        S4RunScheduleError::OptimizerStepBudgetExceeded {
            seed: 0,
            requested_step: 20_001,
            max_step: S4_OPTIMIZER_STEPS_GUTENBERG,
        }
    );
}

#[test]
fn per_seed_run_loop_step_state_is_independent() {
    let config = S4TrainConfig::pinned();
    let mut seed_0 = S4SeedRunLoop::new(0, &config).expect("seed 0 run loop");
    let mut seed_1 = S4SeedRunLoop::new(1, &config).expect("seed 1 run loop");

    assert_eq!(seed_0.seed(), 0);
    assert_eq!(seed_1.seed(), 1);

    assert_eq!(seed_0.advance_one_step().unwrap().step, 1);
    assert_eq!(seed_0.advance_one_step().unwrap().step, 2);
    assert_eq!(seed_0.completed_optimizer_steps(), 2);
    assert_eq!(seed_1.completed_optimizer_steps(), 0);
    assert!(seed_1.event_history().is_empty());

    let seed_1_first = seed_1.advance_one_step().expect("seed 1 first step");
    assert_eq!(seed_1_first.seed, 1);
    assert_eq!(seed_1_first.step, 1);
    assert_eq!(seed_0.advance_one_step().unwrap().step, 3);
}

#[test]
fn s4_step_event_shape_is_subscriber_captured() {
    let config = S4TrainConfig::pinned();
    let mut run_loop = S4SeedRunLoop::new(3, &config).expect("seed run loop");
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        let event = run_loop.advance_one_step().expect("first step emits");
        assert_eq!(event.step, 1);
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == S4_STEP_EVENT_NAME)
        .expect("s4_step event");
    assert_eq!(event.fields.get("seed"), Some(&json!(3)));
    assert_eq!(event.fields.get("step"), Some(&json!(1)));
    assert_eq!(event.fields.get("phase"), Some(&json!("D")));
}

#[test]
fn run_log_lengths_and_progress_eval_steps_match_s4_run_ok_2() {
    let config = S4TrainConfig::pinned();
    let eval_steps = progress_eval_steps(&config).expect("eval steps");

    assert_eq!(required_train_loss_count(), 20_000);
    assert_eq!(eval_steps.len(), 11);
    assert_eq!(
        eval_steps,
        vec![
            0, 2_000, 4_000, 6_000, 8_000, 10_000, 12_000, 14_000, 16_000, 18_000, 20_000
        ]
    );
}

#[test]
fn per_seed_contracts_are_independent_of_requested_order() {
    let config = S4TrainConfig::pinned();
    let forward = seed_run_contracts_for_ordered_seeds(&[0, 1], &config).unwrap();
    let reverse = seed_run_contracts_for_ordered_seeds(&[1, 0], &config).unwrap();
    let forward_by_seed = contracts_by_seed(forward);
    let reverse_by_seed = contracts_by_seed(reverse);

    assert_eq!(forward_by_seed.get(&0), reverse_by_seed.get(&0));
    assert_eq!(forward_by_seed.get(&1), reverse_by_seed.get(&1));
    assert_ne!(
        forward_by_seed.get(&0).unwrap().rng_streams,
        forward_by_seed.get(&1).unwrap().rng_streams
    );
    for contract in forward_by_seed.values() {
        assert_eq!(
            contract.initial_weight_source,
            S4InitialWeightSource::CTsRef
        );
        assert_eq!(
            contract.qat_shadow_weights_initial,
            S4QatShadowWeightSource::CTsRef
        );
        assert_eq!(
            contract.optimizer_state_initial,
            S4OptimizerStateInitial::ZeroInitAdamW
        );
        assert_eq!(contract.phase_state_initial, S4TrainPhase::PhaseD);
        assert_eq!(
            contract.ternary_projection_initial,
            S4TernaryProjectionInitial::PhaseDHardTernaryProjection
        );
        assert_eq!(contract.init_rng_draw_count_before_first_step, 0);
        assert_eq!(contract.shuffle_rng_draw_count_total, 0);
    }
}

#[test]
fn d9_continuation_initialization_loads_cts_weights_and_zeroes_optimizer_state() {
    let inputs = continuation_inputs(2);
    let init = initialize_gutenberg_continuation(&inputs).expect("D9 init");

    assert_eq!(init.seed, 2);
    assert_eq!(
        init.train_config_hash,
        train_config_hash(&inputs.train_config).expect("config hashes")
    );
    assert_eq!(
        init.c_ts_checkpoint_self_hash,
        inputs.c_ts_checkpoint_self_hash
    );
    assert_eq!(
        init.promotion_gate_self_hash,
        inputs.promotion_gate_self_hash
    );
    assert_eq!(
        init.initial_checkpoint_payload_sha,
        inputs.deployed_tensor_payload_sha
    );
    assert_eq!(
        init.initial_fp_shadow_payload_sha,
        inputs.fp_shadow_tensor_payload_sha
    );
    assert_eq!(init.initial_weight_source, S4InitialWeightSource::CTsRef);
    assert_eq!(
        init.qat_shadow_weights_initial,
        S4QatShadowWeightSource::CTsRef
    );
    assert_eq!(
        init.optimizer_state_initial,
        S4OptimizerStateInitial::ZeroInitAdamW
    );
    assert_eq!(init.optimizer_step_initial, 0);
    assert_eq!(init.inherited_adamw_first_moment_payload_sha, None);
    assert_eq!(init.inherited_adamw_second_moment_payload_sha, None);
    assert_eq!(init.phase_state_initial, S4TrainPhase::PhaseD);
    assert_eq!(
        init.ternary_projection_initial,
        S4TernaryProjectionInitial::PhaseDHardTernaryProjection
    );
    assert_eq!(init.init_rng_draw_count_before_first_step, 0);
    assert_eq!(init.batch_rng_draw_count_before_first_step, 0);
    assert_eq!(init.shuffle_rng_draw_count_total, 0);
    assert_eq!(init.rng_streams.batch.domain, S4_BATCH_RNG_DOMAIN);
}

#[test]
fn d9_continuation_initialization_emits_declared_events_with_subscriber_capture() {
    let inputs = continuation_inputs(4);
    let capture = TraceCapture::default();

    let init = with_trace_capture(&capture, || {
        initialize_gutenberg_continuation(&inputs).expect("D9 init")
    });

    let events = captured_events(&capture);
    let started = captured_event(&events, S4_CONTINUATION_INIT_STARTED_EVENT_NAME);
    assert_eq!(started.fields.get("seed"), Some(&json!(4)));
    assert_eq!(
        started.fields.get("c_ts_checkpoint_self_hash"),
        Some(&json!(inputs.c_ts_checkpoint_self_hash.to_string()))
    );
    assert_eq!(
        started.fields.get("deployed_tensor_payload_sha"),
        Some(&json!(inputs.deployed_tensor_payload_sha.to_string()))
    );
    assert_eq!(
        started.fields.get("fp_shadow_tensor_payload_sha"),
        Some(&json!(inputs.fp_shadow_tensor_payload_sha.to_string()))
    );
    assert_eq!(
        started.fields.get("promotion_gate_self_hash"),
        Some(&json!(inputs.promotion_gate_self_hash.to_string()))
    );
    assert_eq!(
        started.fields.get("actual_load_scope"),
        Some(&json!("contract_pin_only"))
    );
    assert_eq!(
        started.fields.get("actual_load_performed"),
        Some(&json!(false))
    );

    let resumed = captured_event(&events, S4_RESUMED_PHASE_D_EVENT_NAME);
    assert_eq!(resumed.fields.get("seed"), Some(&json!(4)));
    assert_eq!(
        resumed.fields.get("train_config_hash"),
        Some(&json!(init.train_config_hash.to_string()))
    );
    assert_eq!(
        resumed.fields.get("initial_checkpoint_payload_sha"),
        Some(&json!(inputs.deployed_tensor_payload_sha.to_string()))
    );
    assert_eq!(
        resumed.fields.get("initial_fp_shadow_payload_sha"),
        Some(&json!(inputs.fp_shadow_tensor_payload_sha.to_string()))
    );
    assert_eq!(
        resumed.fields.get("initial_weight_source"),
        Some(&json!("c_TS_ref"))
    );
    assert_eq!(
        resumed.fields.get("qat_shadow_weights_initial"),
        Some(&json!("c_TS_ref"))
    );
    assert_eq!(
        resumed.fields.get("phase_state_initial"),
        Some(&json!("phase_d"))
    );
    assert_eq!(
        resumed.fields.get("ternary_projection_initial"),
        Some(&json!("phase_d_hard_ternary_projection"))
    );
    assert_eq!(
        resumed.fields.get("actual_load_scope"),
        Some(&json!("contract_pin_only"))
    );
    assert_eq!(
        resumed.fields.get("actual_load_performed"),
        Some(&json!(false))
    );

    let zeroed = captured_event(&events, S4_OPTIMIZER_STATE_ZEROED_EVENT_NAME);
    assert_eq!(zeroed.fields.get("seed"), Some(&json!(4)));
    assert_eq!(
        zeroed.fields.get("optimizer_state_initial"),
        Some(&json!("zero_init_adamw"))
    );
    assert_eq!(
        zeroed.fields.get("optimizer_step_initial"),
        Some(&json!(0_u64))
    );
    assert_eq!(
        zeroed
            .fields
            .get("inherited_adamw_first_moment_payload_sha"),
        Some(&json!("none"))
    );
    assert_eq!(
        zeroed
            .fields
            .get("inherited_adamw_second_moment_payload_sha"),
        Some(&json!("none"))
    );
    assert_eq!(
        zeroed.fields.get("adamw_moment_inheritance_proof"),
        Some(&json!("impossible_by_input_contract"))
    );

    let batch_rng = captured_event(&events, S4_BATCHRNG_INSTANTIATED_EVENT_NAME);
    assert_eq!(batch_rng.fields.get("seed"), Some(&json!(4)));
    assert_eq!(
        batch_rng.fields.get("domain"),
        Some(&json!(S4_BATCH_RNG_DOMAIN))
    );
    assert_eq!(
        batch_rng.fields.get("seed128_hex"),
        Some(&json!(init.rng_streams.batch.seed128_hex.as_str()))
    );
    assert_eq!(
        batch_rng.fields.get("initial_state_hex"),
        Some(&json!(init.rng_streams.batch.initial_state_hex.as_str()))
    );
    assert_eq!(batch_rng.fields.get("draw_count"), Some(&json!(0_u64)));
    assert_eq!(
        batch_rng
            .fields
            .get("batch_rng_draw_count_before_first_step"),
        Some(&json!(0_u64))
    );
}

#[test]
fn d9_continuation_two_cold_initialization_contracts_are_equal() {
    let inputs = continuation_inputs(3);

    let first = initialize_gutenberg_continuation(&inputs).expect("first D9 init");
    let second = initialize_gutenberg_continuation(&inputs).expect("second D9 init");

    assert_eq!(first, second);
    assert_eq!(
        first.optimizer_state_initial,
        S4OptimizerStateInitial::ZeroInitAdamW
    );
    assert_eq!(first.optimizer_step_initial, 0);
    assert_eq!(first.batch_rng_draw_count_before_first_step, 0);
    assert!(first.inherited_adamw_first_moment_payload_sha.is_none());
    assert!(first.inherited_adamw_second_moment_payload_sha.is_none());
}

#[test]
fn d9_continuation_initialization_rejects_missing_lineage_hashes() {
    for (field, mut inputs) in [
        {
            let mut inputs = continuation_inputs(0);
            inputs.c_ts_checkpoint_self_hash = Hash256::ZERO;
            ("c_ts_checkpoint_self_hash", inputs)
        },
        {
            let mut inputs = continuation_inputs(0);
            inputs.deployed_tensor_payload_sha = Hash256::ZERO;
            ("deployed_tensor_payload_sha", inputs)
        },
        {
            let mut inputs = continuation_inputs(0);
            inputs.fp_shadow_tensor_payload_sha = Hash256::ZERO;
            ("fp_shadow_tensor_payload_sha", inputs)
        },
        {
            let mut inputs = continuation_inputs(0);
            inputs.promotion_gate_self_hash = Hash256::ZERO;
            ("promotion_gate_self_hash", inputs)
        },
    ] {
        let error =
            initialize_gutenberg_continuation(&inputs).expect_err("zero lineage hash rejected");
        assert_eq!(error, S4RunScheduleError::MissingInitialHash { field });
        assert_eq!(error.name(), "S4MissingInitialHash");

        inputs.seed = 99;
        assert_eq!(
            initialize_gutenberg_continuation(&inputs).unwrap_err(),
            S4RunScheduleError::Config(S4SchemaError::InvalidSeed { seed: 99 })
        );
    }
}

#[test]
fn s4_rng_streams_pin_d9_domains_and_zero_initial_draw_counts() {
    let streams = S4RngStreams::new(4);

    assert_eq!(streams.seed, 4);
    assert_eq!(streams.init.domain, S4_INIT_RNG_DOMAIN);
    assert_eq!(streams.batch.domain, S4_BATCH_RNG_DOMAIN);
    assert_eq!(streams.shuffle.domain, S4_SHUFFLE_RNG_DOMAIN);
    assert_eq!(streams.init.draw_count, 0);
    assert_eq!(streams.batch.draw_count, 0);
    assert_eq!(streams.shuffle.draw_count, 0);
    assert_eq!(
        streams.init.seed128_hex,
        format!("{:032x}", s4_seed128(S4_INIT_RNG_DOMAIN, 4))
    );
    assert_eq!(
        streams.batch.initial_state_hex,
        format!("{:032x}", s4_seed128(S4_BATCH_RNG_DOMAIN, 4) | 1)
    );
    assert_ne!(
        streams.init.initial_state_hex,
        streams.batch.initial_state_hex
    );
    assert_ne!(
        streams.batch.initial_state_hex,
        streams.shuffle.initial_state_hex
    );

    let json = serde_json::to_value(&streams).expect("streams serialize");
    assert_eq!(json["init"]["draw_count"], json!(0));
    assert_eq!(json["batch"]["domain"], json!("s4-init-batch"));
    assert_eq!(json["shuffle"]["draw_count"], json!(0));
}

#[test]
fn canonical_seed_contracts_require_exact_d11_seed_list() {
    let config = S4TrainConfig::pinned();
    validate_s4_canonical_seed_list(&S4_CANONICAL_SEEDS).unwrap();
    assert!(validate_s4_canonical_seed_list(&[0, 1]).is_err());

    let contracts = canonical_seed_run_contracts(&config).expect("canonical seed contracts");
    assert_eq!(
        contracts
            .iter()
            .map(|contract| contract.seed)
            .collect::<Vec<_>>(),
        S4_CANONICAL_SEEDS
    );
    assert_eq!(
        seed_run_contracts_for_ordered_seeds(&[], &config).unwrap_err(),
        S4RunScheduleError::EmptySeedList
    );
    assert_eq!(
        seed_run_contracts_for_ordered_seeds(&[0, 0], &config).unwrap_err(),
        S4RunScheduleError::DuplicateSeed { seed: 0 }
    );
}

#[test]
fn hypothesis_labels_and_closure_candidate_seed_count_are_pinned() {
    let labels = S4Hypothesis::ALL
        .into_iter()
        .map(|hypothesis| serde_json::to_value(hypothesis).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        vec![
            json!("H1"),
            json!("H2"),
            json!("H3"),
            json!("H4"),
            json!("H5"),
            json!("H6"),
            json!("H7")
        ]
    );

    let bundle = S4VerifierBundle::closure_candidate();
    assert_eq!(bundle.completions.len(), S4_CANONICAL_SEEDS.len());
}

fn mutated_configs(base: &S4TrainConfig) -> Vec<(&'static str, S4TrainConfig)> {
    let mut out = Vec::new();

    let mut config = base.clone();
    config.optimizer_steps += 1;
    out.push(("optimizer_steps", config));

    let mut config = base.clone();
    config.batch_size -= 1;
    out.push(("batch_size", config));

    let mut config = base.clone();
    config.sequence_length -= 1;
    out.push(("sequence_length", config));

    let mut config = base.clone();
    config.eval_every_steps += 1;
    out.push(("eval_every_steps", config));

    let mut config = base.clone();
    config.eval_subset_size -= 1;
    out.push(("eval_subset_size", config));

    let mut config = base.clone();
    config.optimizer.lr = 1.0e-3;
    out.push(("optimizer.lr", config));

    let mut config = base.clone();
    config.optimizer.beta1 = 0.85;
    out.push(("optimizer.beta1", config));

    let mut config = base.clone();
    config.optimizer.beta2 = 0.99;
    out.push(("optimizer.beta2", config));

    let mut config = base.clone();
    config.optimizer.eps = 1.0e-7;
    out.push(("optimizer.eps", config));

    let mut config = base.clone();
    config.optimizer.weight_decay = 0.01;
    out.push(("optimizer.weight_decay", config));

    out
}

fn contracts_by_seed(
    contracts: Vec<gbf_experiments::s4::schema::S4SeedRunContract>,
) -> BTreeMap<u64, gbf_experiments::s4::schema::S4SeedRunContract> {
    contracts
        .into_iter()
        .map(|contract| (contract.seed, contract))
        .collect()
}

fn captured_event<'a>(events: &'a [TracingEvent], name: &str) -> &'a TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| panic!("missing {name}; saw {events:#?}"))
}

fn continuation_inputs(seed: u64) -> S4ContinuationInitInputs {
    S4ContinuationInitInputs {
        seed,
        train_config: S4TrainConfig::pinned(),
        c_ts_checkpoint_self_hash: hash(0x11),
        deployed_tensor_payload_sha: hash(0x22),
        fp_shadow_tensor_payload_sha: hash(0x33),
        promotion_gate_self_hash: hash(0x44),
    }
}

fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
