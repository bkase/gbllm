#![cfg(feature = "s7")]

mod common;

use gbf_artifact::{S7AggregateParityVerdict, S7Topology};
use gbf_experiments::s7::emulator_one_token::{
    ArtifactOracleOneTokenTrace, EmulatorOneTokenComparison, EmulatorOneTokenObservation,
    compare_with_artifact_oracle_trace,
};
use gbf_experiments::s7::schema::{
    EMULATOR_ONE_TOKEN_SCHEMA, EmulatorOneTokenReport, EmulatorOneTokenReportError,
};
use gbf_experiments::s7::smoke::{
    S7_SMOKE_BYTES_DIAGNOSTIC_EVENT, S7_SMOKE_BYTES_EVENT, S7_SMOKE_CLOSURE_EVENT,
    S7_SMOKE_COLLAPSE_DIAGNOSTIC_EVENT, S7_SMOKE_GUARDRAIL_EVENT, S7_SMOKE_OUTCOME_EVENT,
    S7_SMOKE_PARETO_EVENT, S7_SMOKE_PARITY_DIAGNOSTIC_EVENT, S7_SMOKE_PARITY_EVENT,
    S7_SMOKE_REAL_PRODUCER_OWNER, S7_SMOKE_SCHEMA_VERSION, S7_SMOKE_SWITCH_STATS_EVENT,
    S7_SMOKE_TRANSITION_EVENT, S7SmokeRun, S7SmokeScenario, run_s7_smoke, write_s7_smoke_artifacts,
};
use gbf_foundation::Hash256;
use gbf_train::runtime::collapse_halt::{
    CollapseHaltConfig, CollapseHaltDecision, CollapseHaltMonitor, ENTROPY_WINDOW_STEPS,
    ROUTER_COLLAPSE_GRACE_STEPS,
};
use serde_json::json;
use std::path::{Path, PathBuf};

use common::tracing_capture::{TraceCapture, TracingEvent, captured_events, with_trace_capture};

const PHASE_B_START: u64 = 1_000;
const N_EXPERTS: usize = 4;
const N_BLOCKS: u32 = 4;
const LOW_LAYER_ENTROPY_BITS: &[f32] = &[0.25, 0.30, 0.35, 0.40];
const HEALTHY_LAYER_ENTROPY_BITS: &[f32] = &[1.70, 1.80, 1.90, 2.0];
const SMOKE_ROOT: &str = "experiments/S7/smoke";

#[test]
fn e2e_s7_smoke_pass_matches_committed_outputs_and_closure_envelope() {
    let (run, events) = capture_smoke(S7SmokeScenario::PassClean);

    assert_eq!(run.transcript().outcome, "Pass-clean");
    assert_eq!(run.transcript().decision, "ProceedToS8");
    assert!(run.transcript().closure_valid);
    assert_eq!(
        run.transcript().moved_real_producer_scope_to,
        S7_SMOKE_REAL_PRODUCER_OWNER
    );
    assert!(!run.human_report().contains("bank_switches delta=0"));
    assert!(run.human_report().contains("not smoke-measured"));
    assert!(run.human_report().contains("Fixture [synthetic score path"));
    assert!(run.human_report().contains("Moved [bd-2v9r]"));
    run.dense_vs_moe()
        .verify_self_hash()
        .expect("dense-vs-MoE self hash verifies");
    run.collapse_sweep()
        .verify_self_hash()
        .expect("collapse sweep self hash verifies");

    assert_golden("S7-smoke-report.md", run.human_report().as_bytes());
    assert_golden(
        "transcript.v1.json",
        &run.transcript_json_pretty().expect("transcript json"),
    );
    assert_golden(
        "s7_dense_vs_moe.v1.json",
        &run.dense_vs_moe_json_bytes().expect("dense-vs-moe json"),
    );
    assert_golden(
        "s7_router_collapse_sweep.v1.json",
        &run.collapse_sweep_json_bytes()
            .expect("collapse sweep json"),
    );
    assert_golden(
        "structured_events.v1.json",
        &structured_events_json(&events),
    );
}

#[test]
fn e2e_s7_smoke_fail_parity_routes_dense_only_and_emits_debug_diagnostic() {
    let (run, events) = capture_smoke(S7SmokeScenario::FailParity);

    assert_eq!(run.transcript().aggregate_parity, "Fail-parity");
    assert_eq!(run.transcript().outcome, "Fail-parity");
    assert_eq!(run.transcript().decision, "ProceedToS8-DenseOnly");
    assert!(run.transcript().closure_valid);

    let diagnostic = event_named(&events, S7_SMOKE_PARITY_DIAGNOSTIC_EVENT);
    assert_eq!(diagnostic.level, "DEBUG");
    assert_required_common_fields(diagnostic);
    assert_eq!(
        diagnostic.fields.get("n_experts"),
        Some(&json!(2)),
        "diagnostic must carry n_experts parsed from the tiny topology fixture"
    );
    assert!(diagnostic.fields.contains_key("raw_loss_diagnostics"));
    assert!(diagnostic.fields.contains_key("router_grad_norm_l2"));
}

#[test]
fn e2e_s7_smoke_fail_bytes_blocks_dense_only_by_line_12_and_logs_root() {
    let (run, events) = capture_smoke(S7SmokeScenario::FailBytes);

    assert_eq!(run.transcript().aggregate_parity, "Fail-bytes");
    assert_eq!(run.transcript().outcome, "Fail-bytes");
    assert!(
        run.transcript()
            .decision
            .starts_with("Halt(matched-bytes-invalid")
    );
    assert!(!run.transcript().closure_valid);

    let bytes_event = event_named(&events, S7_SMOKE_BYTES_EVENT);
    assert_eq!(bytes_event.fields.get("verdict"), Some(&json!("Fail")));
    let diagnostic = event_named(&events, S7_SMOKE_BYTES_DIAGNOSTIC_EVENT);
    assert_eq!(diagnostic.level, "DEBUG");
    assert_eq!(
        diagnostic.fields.get("moved_real_producer_scope_to"),
        Some(&json!(S7_SMOKE_REAL_PRODUCER_OWNER))
    );
}

#[test]
fn e2e_s7_smoke_fail_collapse_keeps_production_adoption_scope_explicit() {
    let (run, events) = capture_smoke(S7SmokeScenario::FailCollapse);

    assert_eq!(run.transcript().outcome, "Fail-router-collapse");
    assert!(
        run.transcript()
            .decision
            .starts_with("Investigate(reduce-lambda-switch")
    );
    assert!(!run.transcript().closure_valid);
    assert_eq!(
        run.transcript().moved_real_producer_scope_to,
        S7_SMOKE_REAL_PRODUCER_OWNER
    );
    assert_eq!(run.transcript().collapsed_at_step, Some(1_337));
    assert!(
        run.human_report().contains("CollapsedAt(step=1337)"),
        "collapse report must surface the exact fixture collapsed-at step"
    );

    let guardrail = event_named(&events, S7_SMOKE_GUARDRAIL_EVENT);
    assert_eq!(guardrail.fields.get("verdict"), Some(&json!("FailB")));
    let diagnostic = event_named(&events, S7_SMOKE_COLLAPSE_DIAGNOSTIC_EVENT);
    assert_eq!(diagnostic.level, "DEBUG");
    assert!(diagnostic.fields.contains_key("conformance_gap_diagnostic"));
}

#[test]
fn e2e_s7_smoke_artifact_aggregate_reuses_canonical_parity_dispatch() {
    let cases = [
        (
            S7SmokeScenario::PassClean,
            S7AggregateParityVerdict::PassClean,
        ),
        (
            S7SmokeScenario::FailParity,
            S7AggregateParityVerdict::FailParity,
        ),
        (
            S7SmokeScenario::FailBytes,
            S7AggregateParityVerdict::FailBytes,
        ),
    ];

    for (scenario, expected) in cases {
        let (run, _events) = capture_smoke(scenario);
        assert_eq!(run.dense_vs_moe().aggregate_parity_verdict, expected);
        assert_eq!(
            run.transcript().aggregate_parity,
            match expected {
                S7AggregateParityVerdict::PassClean => "Pass-clean",
                S7AggregateParityVerdict::FailParity => "Fail-parity",
                S7AggregateParityVerdict::FailBytes => "Fail-bytes",
            }
        );
    }
}

#[test]
fn e2e_s7_smoke_log_shape_has_dot_notation_and_required_fields() {
    let (_run, events) = capture_smoke(S7SmokeScenario::PassClean);
    let required = [
        S7_SMOKE_TRANSITION_EVENT,
        S7_SMOKE_BYTES_EVENT,
        S7_SMOKE_PARITY_EVENT,
        S7_SMOKE_PARETO_EVENT,
        S7_SMOKE_GUARDRAIL_EVENT,
        S7_SMOKE_SWITCH_STATS_EVENT,
        S7_SMOKE_OUTCOME_EVENT,
        S7_SMOKE_CLOSURE_EVENT,
    ];

    for name in required {
        let event = event_named(&events, name);
        assert!(
            name.starts_with("s7.") && name.contains('.'),
            "{name} must use s7 dot notation"
        );
        assert_required_common_fields(event);
        if name != S7_SMOKE_TRANSITION_EVENT {
            assert!(event.fields.contains_key("verdict"), "{name} verdict");
            assert!(event.fields.contains_key("reason"), "{name} reason");
            assert!(event.fields.contains_key("inputs"), "{name} inputs");
        }
    }
}

#[test]
fn s7_e2e_smoke_script_prints_committed_human_report_shape() {
    let script = repo_root().join("scripts/s7_e2e_smoke.sh");
    let script_text = std::fs::read_to_string(&script).expect("script text");
    assert!(script_text.contains("prints the committed deterministic smoke report"));
    assert!(script_text.contains("does not"));
    assert!(script_text.contains("post-process tracing logs"));
    let output = std::process::Command::new("bash")
        .arg(script)
        .output()
        .expect("s7 smoke script runs");

    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("S7 SMOKE - fixture=tiny_v1 pass_version=1"));
    assert!(stdout.contains("H3 Parity gate"));
    assert!(stdout.contains("Outcome: Pass-clean -> Decision: ProceedToS8"));
}

#[test]
fn h10_emulator_report_compares_bank_switches_against_artifact_oracle_trace() {
    let report = h10_report_from_artifact_oracle_trace();

    assert_eq!(report.schema, EMULATOR_ONE_TOKEN_SCHEMA);
    assert_eq!(report.oracle_recorded_bank_switches, 0.75);
    assert_eq!(report.bank_switch_diff, 0.5);
    assert!(report.bank_switch_within_one);
    report
        .validate_with_n_blocks(N_BLOCKS)
        .expect("valid report");
    report.verify_self_hash().expect("self hash verifies");

    let value = serde_json::to_value(&report).expect("json");
    let object = value.as_object().expect("report json object");
    assert_eq!(value["schema"], json!(EMULATOR_ONE_TOKEN_SCHEMA));
    assert!(object.contains_key("oracle_recorded_bank_switches"));
    assert!(object.contains_key("s5_tolerance"));
    assert!(!object.contains_key("training_recorded_bank_switches"));
    assert!(!object.contains_key("s6_tolerance"));
}

#[test]
fn h10_emulator_report_rejects_stale_training_recorded_field() {
    let report = h10_report_from_artifact_oracle_trace();
    let mut value = serde_json::to_value(&report).expect("json");
    value["training_recorded_bank_switches"] = json!(0.75);

    serde_json::from_value::<EmulatorOneTokenReport>(value)
        .expect_err("stale training-recorded field must not deserialize");
}

#[test]
fn h10_emulator_report_validates_oracle_bank_switch_diff_and_tolerance() {
    let mut report = h10_report_from_artifact_oracle_trace();
    report.bank_switch_diff = 0.25;
    assert!(matches!(
        report.validate_with_n_blocks(N_BLOCKS),
        Err(EmulatorOneTokenReportError::BankSwitchDiffMismatch { .. })
    ));

    let err = compare_with_artifact_oracle_trace(EmulatorOneTokenComparison {
        pairwise_max_abs_diff: 0.02,
        s5_tolerance: 0.01,
        ..h10_comparison()
    })
    .expect_err("pairwise diff above S5 tolerance must fail");
    assert!(matches!(
        err,
        EmulatorOneTokenReportError::PairwiseDiffExceedsS5Tolerance { .. }
    ));
}

#[test]
fn d16_single_noisy_step_during_grace_does_not_halt() {
    let mut monitor = collapse_monitor();
    let grace_step = PHASE_B_START + ROUTER_COLLAPSE_GRACE_STEPS - 1;
    let observation = monitor
        .observe_step(grace_step, LOW_LAYER_ENTROPY_BITS)
        .expect("grace observation");

    assert_eq!(observation.step(), grace_step);
    assert_eq!(observation.entropy_for_step_bits(), 0.25);
    assert_eq!(observation.rolling_mean_bits(), None);
    assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
}

#[test]
fn d16_sustained_collapse_after_grace_halts_with_collapsed_at() {
    let mut monitor = collapse_monitor();
    let first_checked_step = monitor.config().first_checked_step().unwrap();

    for offset in 0..(ENTROPY_WINDOW_STEPS - 1) {
        let observation = monitor
            .observe_step(first_checked_step + offset as u64, LOW_LAYER_ENTROPY_BITS)
            .expect("pre-full collapse window");
        assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    }

    let collapsed_step = first_checked_step + (ENTROPY_WINDOW_STEPS as u64) - 1;
    let observation = monitor
        .observe_step(collapsed_step, LOW_LAYER_ENTROPY_BITS)
        .expect("full collapse window");

    assert_eq!(
        observation.decision(),
        CollapseHaltDecision::CollapsedAt(collapsed_step)
    );
    assert!(
        observation.rolling_mean_bits().unwrap() < monitor.config().entropy_floor_bits(),
        "rolling mean must fall below the D16 floor"
    );
}

#[test]
fn d16_layer_min_catches_one_collapsed_router_layer() {
    let mut monitor = collapse_monitor();
    let first_checked_step = monitor.config().first_checked_step().unwrap();
    let one_collapsed_layer = &[1.75, 1.80, 0.25, 1.90];

    for offset in 0..ENTROPY_WINDOW_STEPS {
        let observation = monitor
            .observe_step(first_checked_step + offset as u64, one_collapsed_layer)
            .expect("layer-min collapse window");
        if offset + 1 < ENTROPY_WINDOW_STEPS {
            assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
        } else {
            assert_eq!(
                observation.decision(),
                CollapseHaltDecision::CollapsedAt(first_checked_step + offset as u64)
            );
            assert_eq!(observation.entropy_for_step_bits(), 0.25);
        }
    }
}

#[test]
fn d16_single_low_dip_with_healthy_window_does_not_halt() {
    let mut monitor = collapse_monitor();
    let first_checked_step = monitor.config().first_checked_step().unwrap();

    for offset in 0..ENTROPY_WINDOW_STEPS {
        let entropy = if offset == 42 {
            LOW_LAYER_ENTROPY_BITS
        } else {
            HEALTHY_LAYER_ENTROPY_BITS
        };
        let observation = monitor
            .observe_step(first_checked_step + offset as u64, entropy)
            .expect("healthy window with one dip");
        assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    }

    let observation = monitor
        .observe_step(
            first_checked_step + ENTROPY_WINDOW_STEPS as u64,
            HEALTHY_LAYER_ENTROPY_BITS,
        )
        .expect("healthy trailing step");

    assert_eq!(observation.decision(), CollapseHaltDecision::Continue);
    assert!(
        observation.rolling_mean_bits().unwrap() > monitor.config().entropy_floor_bits(),
        "single low dip must not pull the rolling window below the floor"
    );
}

fn collapse_monitor() -> CollapseHaltMonitor {
    CollapseHaltMonitor::new(CollapseHaltConfig::new(PHASE_B_START, N_EXPERTS).unwrap())
}

fn h10_report_from_artifact_oracle_trace() -> EmulatorOneTokenReport {
    compare_with_artifact_oracle_trace(h10_comparison())
        .expect("H10 report from artifact-oracle tracer")
}

fn h10_comparison() -> EmulatorOneTokenComparison {
    EmulatorOneTokenComparison {
        seed: 0,
        topology: S7Topology::MoeTiny,
        encoded_rom_sha: Hash256::ZERO,
        prompt_sha: Hash256::ZERO,
        artifact_oracle_trace: ArtifactOracleOneTokenTrace {
            logits_sha: Hash256::ZERO,
            bank_switches_per_token: 0.75,
        },
        emulator_observation: EmulatorOneTokenObservation {
            logits_sha: Hash256::ZERO,
            bank_switches_per_token: 1.25,
        },
        pairwise_max_abs_diff: 0.000_001,
        s5_tolerance: 0.000_01,
        n_blocks: N_BLOCKS,
    }
}

fn capture_smoke(scenario: S7SmokeScenario) -> (S7SmokeRun, Vec<TracingEvent>) {
    let capture = TraceCapture::default();
    let run = with_trace_capture(&capture, || run_s7_smoke(scenario).expect("smoke run"));
    (run, captured_events(&capture))
}

fn event_named<'a>(events: &'a [TracingEvent], name: &str) -> &'a TracingEvent {
    events
        .iter()
        .find(|event| event.name == name)
        .unwrap_or_else(|| panic!("missing event {name}; saw {:?}", event_names(events)))
}

fn event_names(events: &[TracingEvent]) -> Vec<&str> {
    events.iter().map(|event| event.name.as_str()).collect()
}

fn assert_required_common_fields(event: &TracingEvent) {
    assert_eq!(event.fields.get("topology"), Some(&json!("MoeTiny")));
    assert_eq!(event.fields.get("seed"), Some(&json!(0)));
    assert_eq!(event.fields.get("train_step"), Some(&json!(20_000)));
    assert_eq!(
        event.fields.get("schema_version"),
        Some(&json!(S7_SMOKE_SCHEMA_VERSION))
    );
    let prefix = event
        .fields
        .get("self_hash_prefix")
        .and_then(|value| value.as_str())
        .expect("self_hash_prefix string");
    assert_eq!(prefix.len(), 12);
}

fn structured_events_json(events: &[TracingEvent]) -> Vec<u8> {
    let filtered = events
        .iter()
        .filter(|event| event.name.starts_with("s7."))
        .map(|event| {
            json!({
                "name": event.name,
                "level": event.level,
                "fields": event.fields,
            })
        })
        .collect::<Vec<_>>();
    let mut bytes = serde_json::to_vec_pretty(&filtered).expect("structured event json");
    bytes.push(b'\n');
    bytes
}

fn assert_golden(relative: &str, actual: &[u8]) {
    let path = smoke_root().join(relative);
    if update_goldens() {
        write_s7_smoke_artifacts(&smoke_root()).expect("write smoke artifacts");
        if relative == "structured_events.v1.json" {
            std::fs::write(&path, actual).expect("write structured events golden");
        }
    }
    let expected = std::fs::read(&path)
        .unwrap_or_else(|error| panic!("read golden {}: {error}", path.display()));
    assert_eq!(
        String::from_utf8_lossy(actual),
        String::from_utf8_lossy(&expected),
        "golden differed: {}",
        path.display()
    );
}

fn update_goldens() -> bool {
    std::env::var_os("GBF_UPDATE_GOLDENS").is_some()
        || std::env::args().any(|arg| arg == "--update-goldens")
}

fn smoke_root() -> PathBuf {
    repo_root().join(SMOKE_ROOT)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}
