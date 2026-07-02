#![cfg(feature = "s7")]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::sync::{Arc, Mutex};

use gbf_artifact::{ParetoVerdict, S7Topology};
use gbf_experiments::S7_LOG_TARGET;
use gbf_experiments::s7::pareto::{S7ParetoPoint, s7_pareto_verdict};
use gbf_experiments::s7::replay::{
    DeterminismAxisReport, S7_DETERMINISM_DIFF_EVENT, S7_DETERMINISM_ENV_ISOLATION_EVENT,
    S7_DETERMINISM_EVAL_EXPORT_ZERO_DRAWS_EVENT, S7_DETERMINISM_MATCHED_BYTES_PIN_EVENT,
    S7_DETERMINISM_PARETO_TOTALITY_EVENT, S7_DETERMINISM_REPLAY_EVENT,
    S7_DETERMINISM_ROUTER_RNG_EVENT, S7_DETERMINISM_RUN_ORDER_ISOLATION_EVENT,
    S7_DETERMINISM_SUMMARY_EVENT, S7_DETERMINISM_SWEEP_EVENT, S7_DETERMINISM_SWITCH_STATS_EVENT,
    S7_DETERMINISM_TOPOLOGY_SCAFFOLD_EVENT, S7_FULL_CLI_REPLAY_OWNER, S7_FULL_CLOSURE_OWNER,
    compare_bytes_and_emit, compare_replay_products_and_emit, compare_scaffold_fingerprints,
    emit_axis_hashes, emit_determinism_summary, fixture_matched_bytes_pin_bytes,
    fixture_replay_hash, fixture_replay_product, fixture_scaffold_fingerprint,
    fixture_sweep_report, fixture_switch_stats_replay,
};
use gbf_experiments::s7::rng_counting::{
    RouterExecutionMode, recompute_router_replay_sample, router_draw_count_for_mode,
};
use gbf_foundation::{Hash256, sha256};
use serde_json::{Value, json};
use tracing_subscriber::layer::Context;
use tracing_subscriber::prelude::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn rep_s7_1_fixture_replay_byte_identity_emits_axis_event() {
    let (report, events) = capture_events(|| {
        let first = fixture_replay_product(S7Topology::MoeTiny, 0).expect("first replay product");
        let second = fixture_replay_product(S7Topology::MoeTiny, 0).expect("second replay product");

        assert_eq!(first.checkpoint_bytes, second.checkpoint_bytes);
        assert_eq!(first.run_log_bytes, second.run_log_bytes);
        assert_eq!(first.score_bytes, second.score_bytes);
        assert_eq!(first.run_log_self_hash, second.run_log_self_hash);
        assert_eq!(first.score_self_hash, second.score_self_hash);

        compare_replay_products_and_emit(S7_DETERMINISM_REPLAY_EVENT, &first, &second)
    });

    assert!(report.equal);
    assert_axis_event(&events, S7_DETERMINISM_REPLAY_EVENT, "replay");
}

#[test]
fn replay_mismatch_diff_event_carries_side_by_side_artifact_hash_table() {
    let (report, events) = capture_events(|| {
        let original =
            fixture_replay_product(S7Topology::MoeTiny, 0).expect("original replay product");
        let replayed =
            fixture_replay_product(S7Topology::MoeTiny, 1).expect("mismatched replay product");

        compare_replay_products_and_emit(S7_DETERMINISM_REPLAY_EVENT, &original, &replayed)
    });

    assert!(!report.equal);
    let diff = event_named(&events, S7_DETERMINISM_DIFF_EVENT);
    assert_eq!(diff.level, "ERROR");
    assert_eq!(diff.target, S7_LOG_TARGET);
    assert_eq!(diff.fields.get("axis"), Some(&json!("replay")));
    assert_eq!(
        diff.fields.get("source_event_name"),
        Some(&json!(S7_DETERMINISM_REPLAY_EVENT))
    );
    assert_eq!(
        diff.fields.get("hash_table_schema"),
        Some(&json!("s7_determinism_hash_table.v1"))
    );

    let hash_table: Value =
        serde_json::from_str(event_str(diff, "hash_table")).expect("hash table is serialized JSON");
    let rows = hash_table.as_array().expect("hash table is an array");
    let row_by_field = rows
        .iter()
        .map(|row| (row["field"].as_str().expect("field string").to_owned(), row))
        .collect::<BTreeMap<_, _>>();
    for field in ["checkpoint_sha", "run_log_self_hash", "score_self_hash"] {
        let row = row_by_field
            .get(field)
            .unwrap_or_else(|| panic!("missing hash-table field {field}"));
        assert_eq!(row["equal"], json!(false), "{field} should differ");
        assert_hash_value(&row["original_hash"]);
        assert_hash_value(&row["replayed_hash"]);
    }
}

#[test]
fn rep_s7_2_router_rng_dropout_jitter_and_dispatch_are_recomputable() {
    let (report, events) = capture_events(|| {
        let first = recompute_router_replay_sample(0, 17, 2, 4, 0.25, 0.03125)
            .expect("first router sample");
        let second = recompute_router_replay_sample(0, 17, 2, 4, 0.25, 0.03125)
            .expect("second router sample");
        let next_step = recompute_router_replay_sample(0, 18, 2, 4, 0.25, 0.03125)
            .expect("next-step router sample");

        assert_eq!(first, second);
        assert_eq!(first.dropout_draw_count, 4);
        assert_eq!(first.jitter_draw_count, 4);
        assert_eq!(first.total_draw_count, 8);
        assert_ne!(
            first.sample_hash().expect("first sample hash"),
            next_step.sample_hash().expect("next-step sample hash"),
            "changing step must change the recomputable substream sample"
        );

        emit_axis_hashes(
            S7_DETERMINISM_ROUTER_RNG_EVENT,
            first.sample_hash().expect("first sample hash"),
            second.sample_hash().expect("second sample hash"),
        )
    });

    assert!(report.equal);
    assert_axis_event(&events, S7_DETERMINISM_ROUTER_RNG_EVENT, "router_rng");
}

#[test]
fn rep_s7_3_scaffold_fingerprint_differs_only_in_permitted_fields() {
    let (report, events) = capture_events(|| {
        let moe = fixture_scaffold_fingerprint(S7Topology::MoeTiny).expect("moe scaffold");
        let dense =
            fixture_scaffold_fingerprint(S7Topology::MoeTinyDenseMatched).expect("dense scaffold");
        let parity = compare_scaffold_fingerprints(&moe, &dense);

        assert!(parity.is_valid());
        assert_eq!(
            parity.permitted_differences,
            [
                "model_topology_hash",
                "router_config_hash",
                "expert_block_config_hash",
            ]
        );
        assert!(parity.unpermitted_differences.is_empty());

        let proof_hash = sha256(format!("{:?}", parity.permitted_differences).as_bytes());
        emit_axis_hashes(
            S7_DETERMINISM_TOPOLOGY_SCAFFOLD_EVENT,
            proof_hash,
            proof_hash,
        )
    });

    assert!(report.equal);
    assert_axis_event(
        &events,
        S7_DETERMINISM_TOPOLOGY_SCAFFOLD_EVENT,
        "topology_scaffold",
    );
}

#[test]
fn rep_s7_4_fixture_sweep_replays_byte_identically() {
    let (report, events) = capture_events(|| {
        let first = fixture_sweep_report(0).expect("first sweep report");
        let second = fixture_sweep_report(0).expect("second sweep report");
        let first_bytes = first.canonical_json_bytes().expect("first sweep bytes");
        let second_bytes = second.canonical_json_bytes().expect("second sweep bytes");

        assert_eq!(first.sweep_self_hash, second.sweep_self_hash);
        compare_bytes_and_emit(S7_DETERMINISM_SWEEP_EVENT, &first_bytes, &second_bytes)
    });

    assert!(report.equal);
    assert_axis_event(&events, S7_DETERMINISM_SWEEP_EVENT, "sweep");
}

#[test]
fn rep_s7_5_matched_bytes_pin_output_is_deterministic() {
    let (report, events) = capture_events(|| {
        let first = fixture_matched_bytes_pin_bytes().expect("first matched-bytes pin");
        for _ in 0..20 {
            assert_eq!(
                fixture_matched_bytes_pin_bytes().expect("replayed matched-bytes pin"),
                first
            );
        }
        let replayed = fixture_matched_bytes_pin_bytes().expect("final matched-bytes pin replay");
        compare_bytes_and_emit(S7_DETERMINISM_MATCHED_BYTES_PIN_EVENT, &first, &replayed)
    });

    assert!(report.equal);
    assert_axis_event(
        &events,
        S7_DETERMINISM_MATCHED_BYTES_PIN_EVENT,
        "matched_bytes_pin",
    );
}

#[test]
fn rep_s7_6_switch_stats_digest_replay_is_byte_identical_with_moved_aggregate_scope() {
    let (report, events) = capture_events(|| {
        let first = fixture_switch_stats_replay(0).expect("first switch stats replay");
        let second = fixture_switch_stats_replay(0).expect("second switch stats replay");

        assert_eq!(
            first.support_level,
            "digest_only_no_aggregate_s7_switch_stats_v1"
        );
        assert_eq!(first.aggregate_switch_stats_self_hash, None);
        assert_eq!(
            first.moved_scope_owners,
            [S7_FULL_CLI_REPLAY_OWNER, S7_FULL_CLOSURE_OWNER]
        );
        assert_eq!(
            first.temporal_switch_digest_bytes,
            second.temporal_switch_digest_bytes
        );
        assert_eq!(
            first.temporal_switch_digest_hashes,
            second.temporal_switch_digest_hashes
        );

        emit_axis_hashes(
            S7_DETERMINISM_SWITCH_STATS_EVENT,
            first.digest_level_hash(),
            second.digest_level_hash(),
        )
    });

    assert!(report.equal);
    assert_axis_event(&events, S7_DETERMINISM_SWITCH_STATS_EVENT, "switch_stats");
}

#[test]
fn rep_s7_7_pareto_verdict_is_total_for_fixture_frontier_cases() {
    let (report, events) = capture_events(|| {
        let cases = [
            (
                point(1.0, 100),
                point(1.1, 100),
                10,
                ParetoVerdict::MoeDominates,
            ),
            (
                point(1.1, 100),
                point(1.0, 100),
                10,
                ParetoVerdict::DenseDominates,
            ),
            (point(1.0, 100), point(1.0, 100), 10, ParetoVerdict::Tied),
            (
                point(1.0, 110),
                point(1.1, 100),
                10,
                ParetoVerdict::MoeWinsUnderByteEquivalence,
            ),
            (
                point(1.1, 100),
                point(1.0, 110),
                10,
                ParetoVerdict::DenseWinsUnderByteEquivalence,
            ),
            (
                point(1.0, 111),
                point(1.1, 100),
                10,
                ParetoVerdict::Incomparable,
            ),
        ];
        let mut observed = BTreeSet::new();
        for (moe, dense, tolerance, expected) in cases {
            let verdict = s7_pareto_verdict(moe, dense, tolerance).expect("pareto verdict");
            assert_eq!(verdict, expected);
            observed.insert(format!("{verdict:?}"));
        }
        assert_eq!(observed.len(), 6);

        let proof_hash = sha256(format!("{observed:?}").as_bytes());
        emit_axis_hashes(S7_DETERMINISM_PARETO_TOTALITY_EVENT, proof_hash, proof_hash)
    });

    assert!(report.equal);
    assert_axis_event(
        &events,
        S7_DETERMINISM_PARETO_TOTALITY_EVENT,
        "pareto_totality",
    );
}

#[test]
fn d14_eval_and_export_modes_consume_zero_router_rng_draws() {
    let (report, events) = capture_events(|| {
        let train = router_draw_count_for_mode(RouterExecutionMode::Train).expect("train count");
        let eval = router_draw_count_for_mode(RouterExecutionMode::Eval).expect("eval count");
        let export = router_draw_count_for_mode(RouterExecutionMode::Export).expect("export count");

        assert!(train.draw_count > 0);
        assert_eq!(eval.draw_count, 0);
        assert_eq!(export.draw_count, 0);

        let proof_hash = sha256(format!(
            "eval={},export={}",
            eval.draw_count, export.draw_count
        ));
        emit_axis_hashes(
            S7_DETERMINISM_EVAL_EXPORT_ZERO_DRAWS_EVENT,
            proof_hash,
            proof_hash,
        )
    });

    assert!(report.equal);
    assert_axis_event(
        &events,
        S7_DETERMINISM_EVAL_EXPORT_ZERO_DRAWS_EVENT,
        "eval_export_zero_draws",
    );
}

#[test]
fn o8_fixture_replay_ignores_disallowed_environment_inputs() {
    let (report, events) = capture_events(|| {
        let clean = with_env(&[], || {
            fixture_replay_hash(S7Topology::MoeTiny, 0).expect("clean env replay hash")
        });
        let dirty = with_env(
            &[
                ("HOST_CLOCK", "fast"),
                ("GBF_S7_UNPINNED_ENV", "please-ignore-me"),
                ("TZ", "Mars/Olympus"),
            ],
            || fixture_replay_hash(S7Topology::MoeTiny, 0).expect("dirty env replay hash"),
        );

        assert_eq!(clean, dirty);
        emit_axis_hashes(S7_DETERMINISM_ENV_ISOLATION_EVENT, clean, dirty)
    });

    assert!(report.equal);
    assert_axis_event(&events, S7_DETERMINISM_ENV_ISOLATION_EVENT, "env_isolation");
}

#[test]
fn o9_fixture_run_order_preserves_per_seed_hashes() {
    let (report, events) = capture_events(|| {
        let forward = replay_hashes_for_order(&[0, 1]).expect("forward order hashes");
        let reverse = replay_hashes_for_order(&[1, 0]).expect("reverse order hashes");

        assert_eq!(forward, reverse);
        assert_ne!(
            forward.get(&0),
            forward.get(&1),
            "different seeds should not collapse to the same fixture replay hash"
        );

        let proof_hash = sha256(format!("{forward:?}").as_bytes());
        emit_axis_hashes(
            S7_DETERMINISM_RUN_ORDER_ISOLATION_EVENT,
            proof_hash,
            proof_hash,
        )
    });

    assert!(report.equal);
    assert_axis_event(
        &events,
        S7_DETERMINISM_RUN_ORDER_ISOLATION_EVENT,
        "run_order_isolation",
    );
}

#[test]
fn determinism_s7_summary_event_aggregates_all_fixture_axes() {
    let (reports, events) = capture_events(|| {
        let reports = all_axis_reports();
        emit_determinism_summary(&reports);
        reports
    });

    assert_eq!(
        reports.iter().map(|report| report.axis).collect::<Vec<_>>(),
        [
            "replay",
            "router_rng",
            "topology_scaffold",
            "sweep",
            "matched_bytes_pin",
            "switch_stats",
            "pareto_totality",
            "eval_export_zero_draws",
            "env_isolation",
            "run_order_isolation",
        ]
    );
    assert!(reports.iter().all(|report| report.equal));

    let summary = event_named(&events, S7_DETERMINISM_SUMMARY_EVENT);
    assert_eq!(summary.level, "INFO");
    assert_eq!(summary.target, S7_LOG_TARGET);
    assert_eq!(summary.fields.get("axes_passed"), Some(&json!(10)));
    assert_eq!(summary.fields.get("axes_failed"), Some(&json!(0)));
    assert_eq!(summary.fields.get("failing_axes"), Some(&json!([])));
}

fn all_axis_reports() -> Vec<DeterminismAxisReport> {
    let first_replay =
        fixture_replay_product(S7Topology::MoeTiny, 0).expect("first replay product");
    let second_replay =
        fixture_replay_product(S7Topology::MoeTiny, 0).expect("second replay product");
    let replay = compare_replay_products_and_emit(
        S7_DETERMINISM_REPLAY_EVENT,
        &first_replay,
        &second_replay,
    );

    let first_router =
        recompute_router_replay_sample(0, 17, 2, 4, 0.25, 0.03125).expect("first router sample");
    let second_router =
        recompute_router_replay_sample(0, 17, 2, 4, 0.25, 0.03125).expect("second router sample");
    let router_rng = emit_axis_hashes(
        S7_DETERMINISM_ROUTER_RNG_EVENT,
        first_router
            .sample_hash()
            .expect("first router sample hash"),
        second_router
            .sample_hash()
            .expect("second router sample hash"),
    );

    let moe = fixture_scaffold_fingerprint(S7Topology::MoeTiny).expect("moe scaffold");
    let dense =
        fixture_scaffold_fingerprint(S7Topology::MoeTinyDenseMatched).expect("dense scaffold");
    let scaffold_parity = compare_scaffold_fingerprints(&moe, &dense);
    assert!(scaffold_parity.is_valid());
    let scaffold_hash = sha256(format!("{:?}", scaffold_parity.permitted_differences).as_bytes());
    let topology_scaffold = emit_axis_hashes(
        S7_DETERMINISM_TOPOLOGY_SCAFFOLD_EVENT,
        scaffold_hash,
        scaffold_hash,
    );

    let first_sweep = fixture_sweep_report(0).expect("first sweep report");
    let second_sweep = fixture_sweep_report(0).expect("second sweep report");
    let sweep = compare_bytes_and_emit(
        S7_DETERMINISM_SWEEP_EVENT,
        &first_sweep
            .canonical_json_bytes()
            .expect("first sweep bytes"),
        &second_sweep
            .canonical_json_bytes()
            .expect("second sweep bytes"),
    );

    let first_pin = fixture_matched_bytes_pin_bytes().expect("first matched-bytes pin");
    let second_pin = fixture_matched_bytes_pin_bytes().expect("second matched-bytes pin");
    let matched_bytes_pin = compare_bytes_and_emit(
        S7_DETERMINISM_MATCHED_BYTES_PIN_EVENT,
        &first_pin,
        &second_pin,
    );

    let first_switch = fixture_switch_stats_replay(0).expect("first switch stats replay");
    let second_switch = fixture_switch_stats_replay(0).expect("second switch stats replay");
    let switch_stats = emit_axis_hashes(
        S7_DETERMINISM_SWITCH_STATS_EVENT,
        first_switch.digest_level_hash(),
        second_switch.digest_level_hash(),
    );

    let pareto_hash = pareto_totality_hash();
    let pareto_totality = emit_axis_hashes(
        S7_DETERMINISM_PARETO_TOTALITY_EVENT,
        pareto_hash,
        pareto_hash,
    );

    let eval = router_draw_count_for_mode(RouterExecutionMode::Eval).expect("eval count");
    let export = router_draw_count_for_mode(RouterExecutionMode::Export).expect("export count");
    let zero_draws_hash = sha256(format!(
        "eval={},export={}",
        eval.draw_count, export.draw_count
    ));
    let eval_export_zero_draws = emit_axis_hashes(
        S7_DETERMINISM_EVAL_EXPORT_ZERO_DRAWS_EVENT,
        zero_draws_hash,
        zero_draws_hash,
    );

    let clean = with_env(&[], || {
        fixture_replay_hash(S7Topology::MoeTiny, 0).expect("clean env replay hash")
    });
    let dirty = with_env(&[("HOST_CLOCK", "fast")], || {
        fixture_replay_hash(S7Topology::MoeTiny, 0).expect("dirty env replay hash")
    });
    let env_isolation = emit_axis_hashes(S7_DETERMINISM_ENV_ISOLATION_EVENT, clean, dirty);

    let forward = replay_hashes_for_order(&[0, 1]).expect("forward order hashes");
    let reverse = replay_hashes_for_order(&[1, 0]).expect("reverse order hashes");
    assert_eq!(forward, reverse);
    let run_order_hash = sha256(format!("{forward:?}").as_bytes());
    let run_order_isolation = emit_axis_hashes(
        S7_DETERMINISM_RUN_ORDER_ISOLATION_EVENT,
        run_order_hash,
        run_order_hash,
    );

    vec![
        replay,
        router_rng,
        topology_scaffold,
        sweep,
        matched_bytes_pin,
        switch_stats,
        pareto_totality,
        eval_export_zero_draws,
        env_isolation,
        run_order_isolation,
    ]
}

fn pareto_totality_hash() -> Hash256 {
    let cases = [
        (
            point(1.0, 100),
            point(1.1, 100),
            10,
            ParetoVerdict::MoeDominates,
        ),
        (
            point(1.1, 100),
            point(1.0, 100),
            10,
            ParetoVerdict::DenseDominates,
        ),
        (point(1.0, 100), point(1.0, 100), 10, ParetoVerdict::Tied),
        (
            point(1.0, 110),
            point(1.1, 100),
            10,
            ParetoVerdict::MoeWinsUnderByteEquivalence,
        ),
        (
            point(1.1, 100),
            point(1.0, 110),
            10,
            ParetoVerdict::DenseWinsUnderByteEquivalence,
        ),
        (
            point(1.0, 111),
            point(1.1, 100),
            10,
            ParetoVerdict::Incomparable,
        ),
    ];
    let observed = cases
        .into_iter()
        .map(|(moe, dense, tolerance, expected)| {
            let verdict = s7_pareto_verdict(moe, dense, tolerance).expect("pareto verdict");
            assert_eq!(verdict, expected);
            format!("{verdict:?}")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(observed.len(), 6);
    sha256(format!("{observed:?}").as_bytes())
}

fn replay_hashes_for_order(seeds: &[u64]) -> Result<BTreeMap<u64, Hash256>, String> {
    seeds
        .iter()
        .copied()
        .map(|seed| {
            fixture_replay_hash(S7Topology::MoeTiny, seed)
                .map(|hash| (seed, hash))
                .map_err(|error| error.to_string())
        })
        .collect()
}

fn point(median_val_bpc: f64, deployed_bytes_total: u64) -> S7ParetoPoint {
    S7ParetoPoint::new(median_val_bpc, deployed_bytes_total).expect("valid Pareto point")
}

fn assert_axis_event(events: &[CapturedEvent], event_name: &str, axis: &str) {
    let event = event_named(events, event_name);
    assert_eq!(event.level, "INFO");
    assert_eq!(event.target, S7_LOG_TARGET);
    assert_eq!(event.fields.get("axis"), Some(&json!(axis)));
    assert_eq!(event.fields.get("equal"), Some(&json!(true)));
    assert_hash_field(event, "original_hash");
    assert_hash_field(event, "replayed_hash");
}

fn assert_hash_field(event: &CapturedEvent, field: &str) {
    let value = event
        .fields
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing {field} string field"));
    assert!(
        value.starts_with("sha256:"),
        "{field} should carry a Hash256 display string, got {value:?}"
    );
}

fn assert_hash_value(value: &Value) {
    let value = value.as_str().expect("hash value should be a string");
    assert!(
        value.starts_with("sha256:"),
        "hash value should carry a Hash256 display string, got {value:?}"
    );
}

fn event_str<'a>(event: &'a CapturedEvent, field: &str) -> &'a str {
    event
        .fields
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("missing {field} string field"))
}

fn event_named<'a>(events: &'a [CapturedEvent], event_name: &str) -> &'a CapturedEvent {
    events
        .iter()
        .find(|event| event.fields.get("event_name").and_then(Value::as_str) == Some(event_name))
        .unwrap_or_else(|| {
            panic!(
                "missing event {event_name:?}; captured {:?}",
                events
                    .iter()
                    .filter_map(|event| event.fields.get("event_name"))
                    .collect::<Vec<_>>()
            )
        })
}

fn with_env<R>(extra: &[(&str, &str)], f: impl FnOnce() -> R) -> R {
    let _guard = ENV_LOCK.lock().expect("env test lock");
    let original = env::vars_os().collect::<Vec<(OsString, OsString)>>();
    for (key, _) in &original {
        unsafe { env::remove_var(key) };
    }
    for (key, value) in extra {
        unsafe { env::set_var(key, value) };
    }
    let result = f();
    for (key, _) in env::vars_os() {
        unsafe { env::remove_var(key) };
    }
    for (key, value) in original {
        unsafe { env::set_var(key, value) };
    }
    result
}

#[derive(Clone, Debug)]
struct CapturedEvent {
    level: String,
    target: String,
    fields: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Default)]
struct TraceCapture {
    events: Arc<Mutex<Vec<CapturedEvent>>>,
}

impl<S> tracing_subscriber::layer::Layer<S> for TraceCapture
where
    S: tracing::Subscriber,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace capture mutex")
            .push(CapturedEvent {
                level: event.metadata().level().to_string(),
                target: event.metadata().target().to_owned(),
                fields: visitor.fields,
            });
    }
}

fn capture_events<R>(f: impl FnOnce() -> R) -> (R, Vec<CapturedEvent>) {
    let capture = TraceCapture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());
    let result = tracing::subscriber::with_default(subscriber, f);
    let events = capture.events.lock().expect("trace capture mutex").clone();
    (result, events)
}

#[derive(Debug, Default)]
struct FieldVisitor {
    fields: BTreeMap<String, Value>,
}

impl FieldVisitor {
    fn insert(&mut self, field: &tracing::field::Field, value: Value) {
        self.fields.insert(field.name().to_owned(), value);
    }
}

impl tracing::field::Visit for FieldVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        let text = format!("{value:?}");
        if let Ok(value) = serde_json::from_str::<Value>(&text) {
            self.insert(field, value);
        } else {
            self.insert(field, json!(trim_debug_string(&text)));
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.insert(field, json!(value));
    }

    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.insert(field, json!(value));
    }

    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.insert(field, json!(value));
    }

    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.insert(field, json!(value));
    }

    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.insert(field, json!(value));
    }
}

fn trim_debug_string(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|stripped| stripped.strip_suffix('"'))
        .unwrap_or(value)
}
