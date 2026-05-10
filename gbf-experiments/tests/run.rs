use gbf_experiments::s1::rng::{BatchRng, InitRng, S1Rng, ShuffleRng};
use gbf_experiments::s1::run::{
    BatchSampler, BatchSamplerError, DivergedRunProduct, DivergenceEvent, DivergenceObserved,
    GradLogPoint, LossNatsPerByte, RunInputs, RunProduct, RunTestOptions, S1_BATCH_SIZE,
    S1_INTEGRATION_EVAL_EVERY_STEPS, S1_INTEGRATION_OPTIMIZER_STEPS, S1_SEQUENCE_LENGTH,
    S1RunError, TrainBudgetProfile, TrainConfig, s1_train_run_with_environment,
    s1_train_run_with_environment_and_options,
};
use gbf_experiments::s1::schema::{GradNormSummary, RunLog, S1CanonicalJson, S1Completion};
use gbf_foundation::Hash256;
use gbf_policy::model_profile::ModelSizeProfile;
use proptest::prelude::*;
use safetensors::SafeTensors;

#[test]
fn batch_sampler_is_deterministic_for_first_100_batches_seed_0() {
    let corpus = patterned_corpus(4096);
    let cfg = TrainConfig::pinned();
    let mut first = BatchSampler::new(&corpus, &cfg, 0).expect("sampler");
    let mut second = BatchSampler::new(&corpus, &cfg, 0).expect("sampler");

    for step in 1..=100 {
        let left = first.draw_step(step).expect("draw left");
        let right = second.draw_step(step).expect("draw right");
        assert_eq!(left, right, "step {step} differed");
        assert_eq!(left.len(), S1_BATCH_SIZE);
        for (batch_index, sequence) in left.iter().enumerate() {
            assert_eq!(sequence.batch_index, batch_index);
            assert_eq!(sequence.bytes.len(), S1_SEQUENCE_LENGTH);
        }
    }
}

#[test]
fn batch_sampler_does_not_leak_across_init_or_shuffle_streams() {
    let corpus = patterned_corpus(2048);
    let cfg = TrainConfig::pinned();
    let mut reference = BatchSampler::new(&corpus, &cfg, 0).expect("reference sampler");
    let mut interleaved = BatchSampler::new(&corpus, &cfg, 0).expect("interleaved sampler");
    let mut init = InitRng::new(0);
    let mut shuffle = ShuffleRng::new(0);

    for step in 1..=16 {
        assert_eq!(
            interleaved.draw_step(step).expect("interleaved draw"),
            reference.draw_step(step).expect("reference draw")
        );
        for _ in 0..7 {
            let _ = init.next_u64();
            let _ = shuffle.next_u64();
        }
    }
}

#[test]
fn batch_sampler_cross_stream_non_leak_matches_batch_rng_reference() {
    let mut reference = BatchRng::new(4);
    let mut batch = BatchRng::new(4);
    let mut init = InitRng::new(4);
    let mut shuffle = ShuffleRng::new(4);

    for _ in 0..1024 {
        assert_eq!(batch.next_u64(), reference.next_u64());
        let _ = init.next_u64();
        let _ = shuffle.next_u64();
    }
}

#[test]
fn batch_sampler_accepts_corpus_length_equal_to_sequence_length() {
    let corpus = patterned_corpus(S1_SEQUENCE_LENGTH);
    let mut sampler = BatchSampler::new(&corpus, &TrainConfig::pinned(), 0).expect("sampler");

    let batch = sampler.draw_step(1).expect("draw");
    assert_eq!(batch.len(), S1_BATCH_SIZE);
    for sequence in batch {
        assert_eq!(sequence.start_offset, 0);
        assert_eq!(sequence.bytes, corpus);
    }
}

#[test]
fn batch_sampler_rejects_short_corpus_before_draws() {
    let corpus = patterned_corpus(S1_SEQUENCE_LENGTH - 1);
    let error = BatchSampler::new(&corpus, &TrainConfig::pinned(), 0)
        .expect_err("short corpus must fail before sampler construction");

    assert_eq!(
        error,
        BatchSamplerError::TrainingCorpusTooShort {
            corpus_len: S1_SEQUENCE_LENGTH - 1,
            sequence_length: S1_SEQUENCE_LENGTH,
        }
    );
}

#[test]
fn train_config_rejects_test_sequence_length_masquerading_as_production() {
    let corpus = patterned_corpus(S1_SEQUENCE_LENGTH);
    let mut cfg = TrainConfig::pinned();
    cfg.sequence_length = 8;

    let error = BatchSampler::new(&corpus, &cfg, 0)
        .expect_err("non-production sequence length must be explicit and rejected");
    assert_eq!(
        error,
        BatchSamplerError::NonProductionSequenceLength {
            observed: 8,
            required: S1_SEQUENCE_LENGTH,
        }
    );
}

#[test]
fn diverged_run_product_canonical_json_round_trips_without_non_finite_payload() {
    let product = DivergedRunProduct {
        seed: 0,
        run_log: empty_run_log(),
        weight_stats: Vec::new(),
        grad_log: Vec::new(),
        completion: S1Completion::DivergedAt { step: 17 },
        divergence_event: DivergenceEvent {
            step: 17,
            observed: DivergenceObserved::NonFiniteLoss,
            last_finite_loss: Some(LossNatsPerByte::new(2.5).expect("finite loss")),
        },
    };

    let bytes = S1CanonicalJson::to_vec(&product).expect("canonical JSON");
    let text = String::from_utf8(bytes.clone()).expect("utf8 JSON");
    assert!(!text.contains("NaN"));
    assert!(!text.contains("Infinity"));
    assert!(text.contains(r#""last_finite_loss":2.5"#));

    let decoded: DivergedRunProduct = serde_json::from_slice(&bytes).expect("round trip");
    let recoded = S1CanonicalJson::to_vec(&decoded).expect("canonical JSON recodes");
    assert_eq!(recoded, bytes);
}

#[test]
fn different_seeds_produce_different_first_1024_batch_offsets() {
    let corpus = patterned_corpus(8192);
    let cfg = TrainConfig::pinned();
    let mut first = BatchSampler::new(&corpus, &cfg, 0).expect("first sampler");
    let mut second = BatchSampler::new(&corpus, &cfg, 1).expect("second sampler");

    let mut differed = false;
    for step in 1..=32 {
        let left = first.draw_step(step).expect("left draw");
        let right = second.draw_step(step).expect("right draw");
        differed |= left.iter().zip(right.iter()).any(|(left, right)| {
            left.start_offset != right.start_offset || left.bytes != right.bytes
        });
    }

    assert!(
        differed,
        "seed 0 and seed 1 matched for 1024 batch elements"
    );
}

#[test]
fn integration_fixture_run_constructs_checkpoint_and_run_log_artifacts() {
    let product = s1_train_run_with_environment(integration_inputs(0), canonical_env())
        .expect("fixture run completes");
    let RunProduct::Completed(product) = product else {
        panic!("fixture run must complete");
    };

    assert_eq!(product.metadata.schema, "s1_checkpoint.v1");
    assert_eq!(product.metadata.budget_profile, "integration_fixture");
    assert_eq!(product.run_log.schema, "s1_run_log.v1");
    assert_eq!(
        product.run_log.losses.len(),
        S1_INTEGRATION_OPTIMIZER_STEPS as usize
    );
    assert_eq!(
        product.run_log.eval_points.len(),
        (S1_INTEGRATION_OPTIMIZER_STEPS / S1_INTEGRATION_EVAL_EVERY_STEPS + 1) as usize
    );
    assert_eq!(
        product.grad_log.len(),
        S1_INTEGRATION_OPTIMIZER_STEPS as usize
    );
    assert_eq!(
        product.weight_stats.len(),
        product.run_log.eval_points.len()
    );
    assert_grad_summary_matches_log(&product.run_log.final_grad_norms, &product.grad_log);
    assert_eq!(product.metadata.completion, S1Completion::Completed);
    assert_eq!(product.completion, S1Completion::Completed);

    let tensors = SafeTensors::deserialize(&product.final_checkpoint).expect("safetensors");
    let tensor = tensors
        .tensor("toy0.fixture.weight")
        .expect("fixture weight tensor");
    assert_eq!(tensor.shape(), &[4, 4]);
    assert_eq!(tensor.data(), expected_fixture_weight_bytes(0).as_slice());
}

#[test]
fn integration_fixture_retry_is_idempotent() {
    let first = s1_train_run_with_environment(integration_inputs(1), canonical_env())
        .expect("first fixture run");
    let second = s1_train_run_with_environment(integration_inputs(1), canonical_env())
        .expect("second fixture run");

    let (RunProduct::Completed(first), RunProduct::Completed(second)) = (first, second) else {
        panic!("fixture runs must complete");
    };
    assert_eq!(first.final_checkpoint, second.final_checkpoint);
    assert_eq!(
        first.metadata.checkpoint_self_hash,
        second.metadata.checkpoint_self_hash
    );
    assert_eq!(
        first.run_log.run_log_self_hash,
        second.run_log.run_log_self_hash
    );
}

#[cfg(feature = "falsify")]
#[test]
fn divergence_injection_returns_diverged_product_without_nan_json() {
    let product = s1_train_run_with_environment_and_options(
        integration_inputs(0),
        canonical_env(),
        RunTestOptions {
            inject_non_finite_loss_at_step: Some(42),
            inject_non_finite_grad_norm_at_step: None,
            ..RunTestOptions::default()
        },
    )
    .expect("divergence is a run product");

    let RunProduct::Diverged(product) = product else {
        panic!("injected run must diverge");
    };
    assert_eq!(product.completion, S1Completion::DivergedAt { step: 42 });
    assert_eq!(product.divergence_event.step, 42);
    assert_eq!(
        product.divergence_event.observed,
        DivergenceObserved::NonFiniteLoss
    );
    assert_eq!(product.run_log.losses.len(), 41);

    let bytes = S1CanonicalJson::to_vec(&product).expect("canonical JSON");
    let text = String::from_utf8(bytes).expect("utf8 JSON");
    assert!(!text.contains("NaN"));
    assert!(!text.contains("Infinity"));
}

#[cfg(feature = "falsify")]
#[test]
fn grad_norm_divergence_injection_returns_diverged_product_without_inf_json() {
    let product = s1_train_run_with_environment_and_options(
        integration_inputs(0),
        canonical_env(),
        RunTestOptions {
            inject_non_finite_loss_at_step: None,
            inject_non_finite_grad_norm_at_step: Some(7),
            ..RunTestOptions::default()
        },
    )
    .expect("grad divergence is a run product");

    let RunProduct::Diverged(product) = product else {
        panic!("injected run must diverge");
    };
    assert_eq!(product.completion, S1Completion::DivergedAt { step: 7 });
    assert_eq!(
        product.divergence_event.observed,
        DivergenceObserved::NonFiniteGradNorm
    );
    assert_eq!(product.run_log.losses.len(), 6);
    assert_eq!(product.grad_log.len(), 6);
    let recorded = product
        .grad_log
        .iter()
        .map(|point| point.grad_norm_l2)
        .collect::<Vec<_>>();
    let expected_max = recorded.iter().copied().fold(0.0_f32, f32::max);
    let expected_mean = recorded.iter().sum::<f32>() / recorded.len() as f32;
    assert_eq!(
        product.run_log.final_grad_norms.global_l2,
        *recorded.last().expect("last finite grad norm")
    );
    assert_eq!(product.run_log.final_grad_norms.max_l2, expected_max);
    assert_eq!(product.run_log.final_grad_norms.mean_l2, expected_mean);

    let bytes = S1CanonicalJson::to_vec(&product).expect("canonical JSON");
    let text = String::from_utf8(bytes).expect("utf8 JSON");
    assert!(!text.contains("NaN"));
    assert!(!text.contains("Infinity"));
}

#[test]
fn env_enforcement_happens_before_fixture_divergence_path() {
    let error =
        s1_train_run_with_environment(integration_inputs(0), [("BURN_NDARRAY_NUM_THREADS", "2")])
            .expect_err("invalid env must stop before fixture execution");

    assert!(matches!(error, S1RunError::DeviceProfile(_)));
}

#[cfg(feature = "falsify")]
#[test]
fn production_profile_runs_burn_loop_under_reduced_test_seam() {
    let mut inputs = integration_inputs(0);
    inputs.corpus_train = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.corpus_val = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.train_config = TrainConfig::pinned();
    inputs.budget_profile = TrainBudgetProfile::Production;

    let product = s1_train_run_with_environment_and_options(
        inputs,
        canonical_env(),
        RunTestOptions {
            effective_optimizer_steps: Some(2),
            effective_eval_every_steps: Some(1),
            effective_eval_subset_size: Some(1),
            ..RunTestOptions::default()
        },
    )
    .expect("production Burn loop completes under reduced seam");
    let RunProduct::Completed(product) = product else {
        panic!("production run must complete");
    };

    assert_eq!(TrainConfig::pinned().optimizer_steps, 10_000);
    assert_eq!(product.metadata.budget_profile, "production");
    assert_eq!(product.metadata.final_step, 2);
    assert_eq!(product.run_log.losses.len(), 2);
    assert_eq!(product.run_log.eval_points.len(), 3);
    assert_eq!(product.grad_log.len(), 2);
    assert_eq!(product.weight_stats.len(), 3);
    assert_grad_summary_matches_log(&product.run_log.final_grad_norms, &product.grad_log);
    assert!(
        product
            .run_log
            .losses
            .iter()
            .all(|(_, loss)| loss.is_finite())
    );

    let tensors = SafeTensors::deserialize(&product.final_checkpoint).expect("safetensors");
    assert!(
        tensors
            .tensor("toy0.production.embedding_tied.weight")
            .is_ok()
    );
    assert!(tensors.tensor("toy0.fixture.weight").is_err());
}

#[test]
fn production_effective_budget_override_is_rejected_in_normal_builds() {
    let mut inputs = integration_inputs(0);
    inputs.corpus_train = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.corpus_val = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.train_config = TrainConfig::pinned();
    inputs.budget_profile = TrainBudgetProfile::Production;

    let options = RunTestOptions {
        #[cfg(feature = "falsify")]
        inject_non_finite_loss_at_step: None,
        #[cfg(feature = "falsify")]
        inject_non_finite_grad_norm_at_step: None,
        #[cfg(feature = "falsify")]
        zero_gradients: false,
        effective_optimizer_steps: Some(2),
        effective_eval_every_steps: Some(1),
        effective_eval_subset_size: Some(1),
    };

    let result = s1_train_run_with_environment_and_options(inputs, canonical_env(), options);

    #[cfg(not(feature = "falsify"))]
    assert!(matches!(
        result,
        Err(S1RunError::InvalidTestRunOptions {
            field: "production_effective_budget"
        })
    ));

    #[cfg(feature = "falsify")]
    assert!(matches!(result, Ok(RunProduct::Completed(_))));
}

#[cfg(feature = "falsify")]
#[test]
fn production_non_finite_burn_gradient_diverges_same_step_without_grad_norm_override() {
    let mut inputs = integration_inputs(0);
    inputs.corpus_train = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.corpus_val = patterned_corpus(S1_SEQUENCE_LENGTH);
    inputs.train_config = TrainConfig::pinned();
    inputs.budget_profile = TrainBudgetProfile::Production;

    let product = s1_train_run_with_environment_and_options(
        inputs,
        canonical_env(),
        RunTestOptions {
            inject_non_finite_grad_norm_at_step: Some(1),
            effective_optimizer_steps: Some(2),
            effective_eval_every_steps: Some(1),
            effective_eval_subset_size: Some(1),
            ..RunTestOptions::default()
        },
    )
    .expect("production grad divergence is a run product");
    let RunProduct::Diverged(product) = product else {
        panic!("injected production run must diverge");
    };

    assert_eq!(product.completion, S1Completion::DivergedAt { step: 1 });
    assert_eq!(
        product.divergence_event.observed,
        DivergenceObserved::NonFiniteGradNorm
    );
    assert!(product.grad_log.is_empty());

    let bytes = S1CanonicalJson::to_vec(&product).expect("canonical JSON");
    let text = String::from_utf8(bytes).expect("utf8 JSON");
    assert!(!text.contains("NaN"));
    assert!(!text.contains("Infinity"));
}

#[test]
fn run_preconditions_return_typed_errors() {
    let mut invalid_seed = integration_inputs(5);
    let error = s1_train_run_with_environment(invalid_seed.clone(), canonical_env())
        .expect_err("seed 5 is rejected");
    assert!(matches!(
        error,
        S1RunError::InvalidSeed {
            field: "seed",
            observed: 5
        }
    ));

    invalid_seed.seed = 0;
    invalid_seed.model_config =
        ModelSizeProfile::moe_tiny(2).expect("MoeTiny is a registered non-S1 profile");
    let error = s1_train_run_with_environment(invalid_seed.clone(), canonical_env())
        .expect_err("non-S1 production model is rejected");
    assert!(matches!(
        error,
        S1RunError::InvalidModelConfig {
            field: "model_config"
        }
    ));

    let mut invalid_config = integration_inputs(0);
    invalid_config.train_config.batch_size += 1;
    let error = s1_train_run_with_environment(invalid_config, canonical_env())
        .expect_err("unpinned config is rejected");
    assert!(matches!(
        error,
        S1RunError::InvalidTrainConfig {
            field: "train_config",
            budget_profile: TrainBudgetProfile::IntegrationFixture
        }
    ));
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 64,
        failure_persistence: None,
        .. ProptestConfig::default()
    })]

    #[test]
    fn sampled_offsets_stay_in_inclusive_valid_range(corpus_len in S1_SEQUENCE_LENGTH..(1usize << 20), seed in any::<u64>()) {
        let corpus = patterned_corpus(corpus_len);
        let cfg = TrainConfig::pinned();
        let mut sampler = BatchSampler::new(&corpus, &cfg, seed).expect("sampler");
        let max_start = u64::try_from(corpus_len - S1_SEQUENCE_LENGTH).expect("test corpus fits u64");

        for step in 1..=4 {
            let batch = sampler.draw_step(step).expect("draw");
            for sequence in batch {
                prop_assert!(sequence.start_offset <= max_start);
                prop_assert_eq!(sequence.bytes.len(), S1_SEQUENCE_LENGTH);
                let start = usize::try_from(sequence.start_offset).expect("offset fits usize");
                prop_assert_eq!(&sequence.bytes, &corpus[start..start + S1_SEQUENCE_LENGTH]);
            }
        }
    }
}

fn patterned_corpus(len: usize) -> Vec<u8> {
    (0..len).map(|index| (index % 251) as u8).collect()
}

fn integration_inputs(seed: u64) -> RunInputs {
    RunInputs {
        corpus_train: patterned_corpus(512),
        corpus_val: patterned_corpus(384),
        model_config: ModelSizeProfile::toy0(),
        train_config: TrainConfig::integration_fixture(),
        seed,
        budget_profile: TrainBudgetProfile::IntegrationFixture,
    }
}

fn canonical_env() -> [(&'static str, &'static str); 4] {
    [
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "1"),
    ]
}

fn expected_fixture_weight_bytes(seed: u64) -> Vec<u8> {
    let mut weights = initial_fixture_weights(seed);
    let corpus = patterned_corpus(512);
    let config = TrainConfig::integration_fixture();
    let mut sampler = BatchSampler::new(&corpus, &config, seed).expect("fixture sampler");

    for step in 1..=S1_INTEGRATION_OPTIMIZER_STEPS {
        let batch = sampler.draw_step(step).expect("fixture batch");
        let loss = expected_fixture_step_loss(&weights, &batch, step);
        for (index, weight) in weights.iter_mut().enumerate() {
            let grad = (loss * 0.001) + ((step as usize + index) % 7) as f32 * 0.00001;
            *weight -= config.optimizer.lr * grad;
        }
    }

    weights
        .into_iter()
        .flat_map(|value| value.to_bits().to_le_bytes())
        .collect()
}

fn initial_fixture_weights(seed: u64) -> Vec<f32> {
    let mut rng = InitRng::new(seed);
    (0..16)
        .map(|_| {
            let draw = rng.next_u64();
            let centered = (draw as f64 / u64::MAX as f64) * 2.0 - 1.0;
            (centered * 0.01) as f32
        })
        .collect()
}

fn expected_fixture_step_loss(
    weights: &[f32],
    batch: &[gbf_experiments::s1::run::Sequence],
    step: u64,
) -> f32 {
    let byte_mean = batch
        .iter()
        .flat_map(|sequence| sequence.bytes.iter())
        .map(|byte| f32::from(*byte))
        .sum::<f32>()
        / (batch.len() * batch[0].bytes.len()) as f32;
    let weight_mean = weights.iter().copied().sum::<f32>() / weights.len() as f32;
    let decay = 1.0 / (1.0 + step as f32 * 0.01);
    (2.0 + byte_mean / 255.0 + weight_mean.abs() * 0.1) * decay
}

fn empty_run_log() -> RunLog {
    RunLog {
        schema: "s1_run_log.v1".to_owned(),
        seed: 0,
        train_config_hash: Hash256::ZERO,
        losses: Vec::new(),
        eval_points: Vec::new(),
        final_grad_norms: GradNormSummary {
            global_l2: 0.0,
            max_l2: 0.0,
            mean_l2: 0.0,
        },
        run_log_self_hash: Hash256::ZERO,
    }
}

fn assert_grad_summary_matches_log(summary: &GradNormSummary, grad_log: &[GradLogPoint]) {
    let last = grad_log.last().expect("completed run has grad entries");
    let expected_max = grad_log
        .iter()
        .map(|point| point.grad_norm_l2)
        .fold(0.0_f32, f32::max);
    let expected_mean =
        grad_log.iter().map(|point| point.grad_norm_l2).sum::<f32>() / grad_log.len() as f32;

    assert_eq!(summary.global_l2, last.grad_norm_l2);
    assert_eq!(summary.max_l2, expected_max);
    assert_eq!(summary.mean_l2, expected_mean);
}
