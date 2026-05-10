//! F-S1.27 tiny-fixture smoke gate.
//!
//! This is the fast PR-cycle smoke, not the full TinyStories closure run. On a
//! local single CPU thread this suite is expected to stay well under 5 minutes.
//! The replay smoke emits `duration_seconds` in
//! `s1.integration_smoke.scenario.complete`; bd-2364 measured the focused
//! target inside this budget, and future runs record their exact wall-clock in
//! the captured event.

mod common;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_experiments::s1::logging::{
    IntegrationSmokeScenarioCompleteEvent, IntegrationSmokeScenarioStartEvent, S1LogEmitter, event,
};
use gbf_experiments::s1::schema::{CheckpointMetadata, RunLog, S1Completion};
use gbf_foundation::{Hash256, sha256};
use serde_json::{Value, json};

const MANIFEST: &str = "gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml";
const REPLAY_SCENARIO: &str = "tiny_fixture_replay_5_seed_integration_fixture";

#[test]
fn tiny_fixture_o2_o9_scripts_pass_in_fast_mode() {
    let gbf_bin = build_gbf_cli();

    run_script(
        "scripts/s1_determinism_check.sh",
        &[
            "--fast",
            "--gbf-bin",
            gbf_bin.to_str().expect("utf8 path"),
            "--json",
        ],
    );
    run_script(
        "scripts/s1_isolation_check.sh",
        &[
            "--fast",
            "--gbf-bin",
            gbf_bin.to_str().expect("utf8 path"),
            "--json",
        ],
    );
}

#[test]
fn tiny_fixture_replay_writes_self_hashed_artifacts_for_five_seeds() {
    let gbf_bin = build_gbf_cli();
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("S1");
    let capture = TraceCapture::default();
    let started = Instant::now();
    let output = with_trace_capture(&capture, || {
        let emitter = S1LogEmitter::new();
        emitter
            .integration_smoke_scenario_start(&IntegrationSmokeScenarioStartEvent {
                scenario: REPLAY_SCENARIO.to_owned(),
                budget_profile: "integration_fixture".to_owned(),
                n_seeds: 5,
            })
            .expect("integration smoke start event");
        run_gbf(
            &gbf_bin,
            &[
                "s1",
                "replay",
                "--manifest",
                MANIFEST,
                "--pass-version",
                "0.1.0",
                "--seed-list",
                "0,1,2,3,4",
                "--device-profile",
                "S1CpuDeterministic",
                "--budget-profile",
                "integration-fixture",
                "--allow-noncanonical-integration-fixture",
                "--out-dir",
                out_dir.to_str().expect("utf8 path"),
            ],
        )
    });
    let summary: Value = serde_json::from_slice(&output).expect("replay JSON summary");
    assert_eq!(summary["budget_profile"], "IntegrationFixture");

    let mut final_safetensors_hashes = BTreeSet::<Hash256>::new();
    for seed in 0..=4 {
        let checkpoint_path = out_dir
            .join("checkpoints")
            .join(format!("seed-{seed}"))
            .join("final.safetensors");
        let metadata_path = out_dir
            .join("checkpoints")
            .join(format!("seed-{seed}"))
            .join("metadata.json");
        let run_log_path = out_dir
            .join("runs")
            .join(format!("seed-{seed}"))
            .join("run_log.json");

        assert!(checkpoint_path.exists(), "missing {checkpoint_path:?}");
        assert!(metadata_path.exists(), "missing {metadata_path:?}");
        assert!(run_log_path.exists(), "missing {run_log_path:?}");

        let metadata: CheckpointMetadata =
            read_json(&metadata_path).unwrap_or_else(|error| panic!("{metadata_path:?}: {error}"));
        let run_log: RunLog =
            read_json(&run_log_path).unwrap_or_else(|error| panic!("{run_log_path:?}: {error}"));

        assert_eq!(metadata.seed, seed);
        assert_eq!(metadata.budget_profile, "integration_fixture");
        assert_eq!(metadata.completion, S1Completion::Completed);
        assert_eq!(
            metadata.checkpoint_self_hash,
            metadata.computed_self_hash().expect("checkpoint self hash")
        );
        assert_eq!(run_log.seed, seed);
        assert_eq!(
            run_log.run_log_self_hash,
            run_log.computed_self_hash().expect("run log self hash")
        );
        final_safetensors_hashes.insert(sha256(
            std::fs::read(&checkpoint_path)
                .unwrap_or_else(|error| panic!("{checkpoint_path:?}: {error}")),
        ));
    }

    assert!(
        final_safetensors_hashes.len() >= 2,
        "O9 smoke failed: all five fixture seeds produced the same final.safetensors bytes"
    );
    with_trace_capture(&capture, || {
        S1LogEmitter::new()
            .integration_smoke_scenario_complete(&IntegrationSmokeScenarioCompleteEvent {
                scenario: REPLAY_SCENARIO.to_owned(),
                pass: true,
                duration_seconds: started.elapsed().as_secs_f64(),
            })
            .expect("integration smoke complete event");
    });
    assert_integration_smoke_events(&capture);
}

fn build_gbf_cli() -> PathBuf {
    let status = Command::new("cargo")
        .args(["build", "-p", "gbf-cli"])
        .current_dir(repo_root())
        .status()
        .expect("build gbf-cli");
    assert!(status.success(), "cargo build -p gbf-cli failed");
    repo_root().join("target/debug/gbf-cli")
}

fn run_script(script: &str, args: &[&str]) {
    let status = Command::new(repo_root().join(script))
        .args(args)
        .current_dir(repo_root())
        .status()
        .expect("run script");
    assert!(status.success(), "{script} failed");
}

fn run_gbf(gbf_bin: &Path, args: &[&str]) -> Vec<u8> {
    let output = Command::new(gbf_bin)
        .args(args)
        .current_dir(repo_root())
        .env_clear()
        .env("BURN_NDARRAY_NUM_THREADS", "1")
        .env("BURN_DETERMINISTIC", "1")
        .env("OMP_NUM_THREADS", "1")
        .env("RAYON_NUM_THREADS", "1")
        .output()
        .expect("run gbf-cli");
    assert!(
        output.status.success(),
        "gbf-cli failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn read_json<T>(path: &Path) -> Result<T, Box<dyn std::error::Error>>
where
    T: serde::de::DeserializeOwned,
{
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}

fn assert_integration_smoke_events(capture: &TraceCapture) {
    let events = captured_events(capture)
        .into_iter()
        .filter(|event| {
            matches!(
                event.name.as_str(),
                event::INTEGRATION_SMOKE_SCENARIO_START
                    | event::INTEGRATION_SMOKE_SCENARIO_COMPLETE
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 2, "integration smoke events: {events:?}");
    assert_eq!(events[0].name, event::INTEGRATION_SMOKE_SCENARIO_START);
    assert_eq!(
        events[0].fields.get("scenario"),
        Some(&json!(REPLAY_SCENARIO))
    );
    assert_eq!(
        events[0].fields.get("budget_profile"),
        Some(&json!("integration_fixture"))
    );
    assert_eq!(events[0].fields.get("n_seeds"), Some(&json!(5)));
    assert_eq!(events[1].name, event::INTEGRATION_SMOKE_SCENARIO_COMPLETE);
    assert_eq!(
        events[1].fields.get("scenario"),
        Some(&json!(REPLAY_SCENARIO))
    );
    assert_eq!(events[1].fields.get("pass"), Some(&json!(true)));
    let duration = events[1]
        .fields
        .get("duration_seconds")
        .and_then(Value::as_f64)
        .expect("duration_seconds field");
    assert!(duration.is_finite() && duration >= 0.0, "{duration}");
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}
