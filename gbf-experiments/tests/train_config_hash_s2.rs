use std::collections::BTreeSet;

mod common;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s2::environment::S2EnvironmentHash;
use gbf_experiments::s2::schema::{
    PhaseKindS2, S2_LAMBDA_RANGE, S2_LAMBDA_ZERO, S2_OPTIMIZER_STEPS, S2_RANGE_SAFE_HI,
    S2_RANGE_SAFE_LO, S2_TEACHER_FREEZE_STEP, S2_THRESHOLD_INIT_MULTIPLIER, S2BuildKind,
    S2SchemaError, TrainConfigS2Full, TrainingLossUnit, phase_a_effective_config_hash,
    train_config_hash, train_config_hash_with_environment_hash,
};
use gbf_foundation::Hash256;
use serde_json::json;

#[test]
fn canonical_train_config_s2_full_pins_rfc_defaults() {
    let config = TrainConfigS2Full::pinned();

    assert_eq!(config.optimizer_steps, S2_OPTIMIZER_STEPS);
    assert_eq!(config.batch_size, 32);
    assert_eq!(config.sequence_length, 128);
    assert_eq!(config.eval_every_steps, 1000);
    assert_eq!(config.eval_subset_size, 4096);
    assert_eq!(config.phase_plan.len(), 4);
    assert_eq!(config.phase_plan[0].phase, PhaseKindS2::PhaseA);
    assert_eq!(config.phase_plan[0].start_step, 1);
    assert_eq!(config.phase_plan[0].end_step, S2_TEACHER_FREEZE_STEP);
    assert_eq!(config.phase_plan[3].phase, PhaseKindS2::PhaseD);
    assert_eq!(config.phase_plan[3].end_step, S2_OPTIMIZER_STEPS);
    assert_eq!(config.lambda_range, S2_LAMBDA_RANGE);
    assert_eq!(config.lambda_zero, S2_LAMBDA_ZERO);
    assert_eq!(config.range_safe_lo, S2_RANGE_SAFE_LO);
    assert_eq!(config.range_safe_hi, S2_RANGE_SAFE_HI);
    assert_eq!(
        config.threshold_init_multiplier,
        S2_THRESHOLD_INIT_MULTIPLIER
    );
    assert_eq!(config.training_loss_unit, TrainingLossUnit::Nats);
    config.validate().expect("canonical config validates");
}

#[test]
fn train_config_hash_is_deterministic_and_build_kind_sensitive() {
    let config = TrainConfigS2Full::pinned();

    let ternary_a = train_config_hash(&config, S2BuildKind::s2_ternary_full).unwrap();
    let ternary_b = train_config_hash(&config, S2BuildKind::s2_ternary_full).unwrap();
    let fp = train_config_hash(&config, S2BuildKind::s2_fp_full).unwrap();
    let nodistill = train_config_hash(&config, S2BuildKind::s2_ternary_nodistill).unwrap();

    assert_eq!(ternary_a, ternary_b);
    assert_ne!(ternary_a, fp);
    assert_ne!(ternary_a, nodistill);
    assert_ne!(fp, nodistill);
}

#[test]
fn train_config_hash_fixtures_snapshot_pins_build_kind_table() {
    let config = TrainConfigS2Full::pinned();
    let environment_hash = S2EnvironmentHash {
        build_config_hash: hash(11),
        rust_toolchain_hash: hash(12),
        dependency_lockfile_hash: hash(13),
    };
    let rows = [
        S2BuildKind::s2_ternary_full,
        S2BuildKind::s2_fp_full,
        S2BuildKind::s2_ternary_nodistill,
        S2BuildKind::s2_ablation,
    ]
    .into_iter()
    .map(|build_kind| {
        json!({
            "build_kind": build_kind,
            "train_config_hash": train_config_hash_with_environment_hash(
                &config,
                build_kind,
                environment_hash,
            )
            .unwrap()
            .to_string(),
        })
    })
    .collect::<Vec<_>>();
    let bytes = S1CanonicalJson::value_to_vec(&json!(rows)).unwrap();

    insta::assert_snapshot!(
        "buildkind_s2__train_config_hash_fixtures",
        String::from_utf8(bytes).unwrap()
    );
}

#[test]
fn train_config_built_event_shape_is_captured() {
    let config = TrainConfigS2Full::pinned();
    let capture = TraceCapture::default();

    with_trace_capture(&capture, || {
        let _ = train_config_hash(&config, S2BuildKind::s2_fp_full).unwrap();
    });

    let events = captured_events(&capture);
    let event = events
        .iter()
        .find(|event| event.name == "train_config_built")
        .expect("train_config_built event");
    assert_eq!(event.fields.get("build_kind"), Some(&json!("s2_fp_full")));
    assert!(event.fields.contains_key("train_config_hash"));
    assert!(event.fields.contains_key("phase_a_effective_hash"));
    assert_eq!(event.fields.get("training_loss_unit"), Some(&json!("nats")));
}

#[test]
fn changing_each_rep_s2_4_field_changes_train_config_hash() {
    let base = TrainConfigS2Full::pinned();
    let base_hash = train_config_hash(&base, S2BuildKind::s2_ternary_full).unwrap();
    let rejected_noncanonical_fields = BTreeSet::from(["phase_plan", "distill_temp"]);
    let mut observed_fields = BTreeSet::new();
    let mut rejected_fields = BTreeSet::new();

    for (field, mutated) in mutated_configs(&base) {
        assert!(observed_fields.insert(field), "{field} listed twice");
        match train_config_hash(&mutated, S2BuildKind::s2_ternary_full) {
            Ok(hash) => assert_ne!(hash, base_hash, "{field} must perturb train_config_hash"),
            Err(_) => {
                rejected_fields.insert(field);
                assert!(
                    rejected_noncanonical_fields.contains(field),
                    "{field} should hash or be rejected as non-canonical"
                );
            }
        }
    }
    assert_eq!(rejected_fields, rejected_noncanonical_fields);
}

#[test]
fn changing_environment_hash_changes_train_config_hash() {
    let config = TrainConfigS2Full::pinned();
    let base_env = S2EnvironmentHash {
        build_config_hash: hash(1),
        rust_toolchain_hash: hash(2),
        dependency_lockfile_hash: hash(3),
    };
    let base =
        train_config_hash_with_environment_hash(&config, S2BuildKind::s2_ternary_full, base_env)
            .unwrap();

    let mutations = [
        S2EnvironmentHash {
            build_config_hash: hash(4),
            ..base_env
        },
        S2EnvironmentHash {
            rust_toolchain_hash: hash(5),
            ..base_env
        },
        S2EnvironmentHash {
            dependency_lockfile_hash: hash(6),
            ..base_env
        },
    ];

    for environment_hash in mutations {
        let changed = train_config_hash_with_environment_hash(
            &config,
            S2BuildKind::s2_ternary_full,
            environment_hash,
        )
        .unwrap();
        assert_ne!(base, changed);
    }
}

#[test]
fn lambda_distill_default_is_excluded_only_from_phase_a_effective_hash() {
    let base = TrainConfigS2Full::pinned();
    let mut changed = base.clone();
    changed.lambda_distill_default = 0.5;

    assert_eq!(
        phase_a_effective_config_hash(&base).unwrap(),
        phase_a_effective_config_hash(&changed).unwrap()
    );
    assert_ne!(
        train_config_hash(&base, S2BuildKind::s2_ternary_full).unwrap(),
        train_config_hash(&changed, S2BuildKind::s2_ternary_full).unwrap()
    );
}

#[test]
fn training_loss_unit_is_present_in_canonical_hash_input() {
    let value = serde_json::to_value(TrainConfigS2Full::pinned()).unwrap();

    assert_eq!(value["training_loss_unit"], "nats");
    let bytes = S1CanonicalJson::value_to_vec(&value).unwrap();
    assert!(
        bytes
            .windows(br#""training_loss_unit":"nats""#.len())
            .any(|window| window == br#""training_loss_unit":"nats""#)
    );
}

#[test]
fn train_config_validation_rejects_invalid_public_values() {
    let mut invalid = TrainConfigS2Full::pinned();
    invalid.distill_temp = 0.0;
    assert!(matches!(
        invalid.validate(),
        Err(S2SchemaError::NonPositiveF32 {
            field: "distill_temp",
            value: 0.0,
        })
    ));

    let mut invalid = TrainConfigS2Full::pinned();
    invalid.range_safe_lo = 2.0;
    assert!(matches!(
        invalid.validate(),
        Err(S2SchemaError::InvalidRangeBounds { lo: 2.0, hi: 1.0 })
    ));
}

#[test]
fn ten_thousand_mutated_train_configs_do_not_collide() {
    let mut seen = BTreeSet::new();
    for index in 0..10_000_u32 {
        let mut config = TrainConfigS2Full::pinned();
        config.lambda_range = (index as f32 + 1.0) / 1_000_000.0;
        config.lambda_zero = ((index % 997) as f32 + 1.0) / 10_000_000.0;
        config.threshold_init_multiplier = 0.5 + (index as f32 / 20_000.0);
        let hash = train_config_hash(&config, S2BuildKind::s2_ternary_full).unwrap();
        assert!(seen.insert(hash), "hash collision at config index {index}");
    }
}

fn mutated_configs(base: &TrainConfigS2Full) -> Vec<(&'static str, TrainConfigS2Full)> {
    let mut out = Vec::new();

    let mut config = base.clone();
    config.optimizer_steps = 10_001;
    out.push(("optimizer_steps", config));

    let mut config = base.clone();
    config.batch_size = 16;
    out.push(("batch_size", config));

    let mut config = base.clone();
    config.sequence_length = 64;
    out.push(("sequence_length", config));

    let mut config = base.clone();
    config.eval_every_steps = 500;
    out.push(("eval_every_steps", config));

    let mut config = base.clone();
    config.eval_subset_size = 2048;
    out.push(("eval_subset_size", config));

    let mut config = base.clone();
    config.optimizer.lr = 0.0005;
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

    let mut config = base.clone();
    config.phase_plan[2].end_step -= 1;
    out.push(("phase_plan", config));

    let mut config = base.clone();
    config.lambda_distill_default = 0.5;
    out.push(("lambda_distill_default", config));

    let mut config = base.clone();
    config.lambda_range = 0.02;
    out.push(("lambda_range", config));

    let mut config = base.clone();
    config.lambda_zero = 0.0002;
    out.push(("lambda_zero", config));

    let mut config = base.clone();
    config.range_safe_lo = -2.0;
    out.push(("range_safe_lo", config));

    let mut config = base.clone();
    config.range_safe_hi = 2.0;
    out.push(("range_safe_hi", config));

    let mut config = base.clone();
    config.threshold_init_multiplier = 0.5;
    out.push(("threshold_init_multiplier", config));

    let mut config = base.clone();
    config.teacher_freeze_step = 3999;
    out.push(("teacher_freeze_step", config));

    let mut config = base.clone();
    config.distill_temp = 1.0;
    out.push(("distill_temp", config));

    let mut config = base.clone();
    config.device_profile.thread_count = 2;
    out.push(("device_profile.thread_count", config));

    out
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
