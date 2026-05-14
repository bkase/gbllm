use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-codegen has workspace parent")
}

fn run_script_with_env<const N: usize>(script: &str, envs: [(&str, &str); N]) -> PathBuf {
    let mut command = Command::new(repo_root().join(script));
    command.current_dir(repo_root());
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command
        .output()
        .unwrap_or_else(|error| panic!("failed to run {script}: {error}"));
    if !output.status.success() {
        panic!(
            "{script} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stdout = String::from_utf8(output.stdout).expect("script stdout is UTF-8");
    repo_root().join(stdout.lines().last().unwrap_or_default())
}

fn read_jsonl(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()))
        .lines()
        .map(|line| serde_json::from_str(line).expect("jsonl record parses"))
        .collect()
}

fn count_event(records: &[Value], event: &str) -> usize {
    records
        .iter()
        .filter(|record| record["event"] == event)
        .count()
}

#[test]
fn chunk2_e2e_pipeline_passes_dense_routed_mixed_and_delegated_gates() {
    let records = read_jsonl(&run_script_with_env(
        "scripts/e2e/chunk2_pipeline.sh",
        [("CHUNK2_E2E_VERBOSE", "1")],
    ));

    assert_eq!(count_event(&records, "chunk2.pipeline.golden.match"), 9);
    assert_eq!(count_event(&records, "chunk2.pipeline.debug"), 1);
    assert!(
        records
            .iter()
            .all(|record| { !record.to_string().contains("current_fixture_api") })
    );
    for fixture in [
        "dense_toy0",
        "dense_toy1_tied",
        "dense_toy1_untied",
        "routed_basic_one",
        "routed_basic_selected_score",
        "mixed_topology",
        "routed_basic",
    ] {
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "chunk2.pipeline.complete"
                    && record["fixture"] == fixture
                    && record["all_stages_passed"] == true),
            "missing successful pipeline record for {fixture}"
        );
    }
    for gate in [
        "stage1_cache_hit_byte_identical",
        "stage3_cache_hit_audit_rewrap",
        "stage3_semantic_equivalence_bit_exact",
    ] {
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "chunk2.pipeline.gate.complete"
                    && record["gate"] == gate
                    && record["status"] == "passed"),
            "missing delegated gate evidence for {gate}"
        );
    }
    assert!(records.iter().all(|record| {
        record
            .get("report_self_hash")
            .and_then(Value::as_str)
            .map(|hash| hash.starts_with("sha256:"))
            .unwrap_or(true)
    }));
}

#[test]
fn chunk2_e2e_manifest_declares_delegated_golden_boundary() {
    let manifest_path = repo_root().join("docs/review/chunk2/golden/manifest.json");
    let manifest: Value = serde_json::from_str(
        &std::fs::read_to_string(&manifest_path).expect("manifest is readable"),
    )
    .expect("manifest parses");

    assert_eq!(
        manifest["current_pipeline_mode"],
        "exported_feature_goldens_plus_driver_gates"
    );
    assert_eq!(
        manifest["chunk2_owned_golden_files"],
        serde_json::json!(["manifest.json"])
    );
    assert_eq!(manifest["passing_quant_graph_fixture_count"], 6);
    assert_eq!(manifest["passing_infer_ir_fixture_count"], 3);
    for gate in [
        "stage1_cache_hit_byte_identical",
        "stage3_cache_hit_audit_rewrap",
        "stage3_semantic_equivalence_bit_exact",
        "infer_ir_reject_taxonomy",
        "quant_graph_reject_taxonomy",
    ] {
        assert!(
            manifest["delegated_gate_evidence"]
                .as_array()
                .expect("delegated gates are an array")
                .iter()
                .any(|record| record["gate"] == gate),
            "manifest missing delegated gate {gate}"
        );
    }
}

#[test]
fn chunk2_e2e_pipeline_missing_golden_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(repo_root().join("scripts/e2e/chunk2_pipeline.sh"))
        .current_dir(repo_root())
        .env("CHUNK2_E2E_SKIP_GATES", "1")
        .env("CHUNK2_E2E_F_B3_GOLDEN_DIR", temp.path())
        .output()
        .expect("pipeline script runs");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing F-B3 quant_graph golden"),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn chunk2_e2e_pipeline_missing_f_b5_golden_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = Command::new(repo_root().join("scripts/e2e/chunk2_pipeline.sh"))
        .current_dir(repo_root())
        .env("CHUNK2_E2E_SKIP_GATES", "1")
        .env("CHUNK2_E2E_F_B5_GOLDEN_DIR", temp.path())
        .output()
        .expect("pipeline script runs");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("missing F-B5 infer_ir golden"),
        "stderr was: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn chunk2_e2e_pipeline_is_idempotent_after_stripping_timestamps() {
    let first = read_jsonl(&run_script_with_env(
        "scripts/e2e/chunk2_pipeline.sh",
        [("CHUNK2_E2E_SKIP_GATES", "1")],
    ));
    let second = read_jsonl(&run_script_with_env(
        "scripts/e2e/chunk2_pipeline.sh",
        [("CHUNK2_E2E_SKIP_GATES", "1")],
    ));

    assert_eq!(normalize_records(first), normalize_records(second));
}

fn normalize_records(records: Vec<Value>) -> Vec<Value> {
    records
        .into_iter()
        .map(|mut record| {
            let object = record.as_object_mut().expect("jsonl record is object");
            object.remove("ts");
            object.remove("total_ms");
            record
        })
        .collect()
}

#[test]
fn chunk2_e2e_reject_script_checks_72_typed_classes_with_gate_evidence() {
    let records = read_jsonl(&run_script_with_env(
        "scripts/e2e/chunk2_reject_classes.sh",
        [("CHUNK2_E2E_VERBOSE", "1")],
    ));

    assert_eq!(count_event(&records, "chunk2.reject.debug"), 1);
    assert_eq!(count_event(&records, "chunk2.reject.expected_class"), 72);
    assert_eq!(count_event(&records, "chunk2.reject.complete"), 1);
    for gate in ["infer_ir_reject_taxonomy", "quant_graph_reject_taxonomy"] {
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "chunk2.reject.gate.complete"
                    && record["gate"] == gate
                    && record["status"] == "passed"),
            "missing reject gate evidence for {gate}"
        );
    }
    assert!(records.iter().all(|record| {
        !record.to_string().contains("generic string diagnostic")
            && !record.to_string().contains("String")
            && record.get("observed_class").is_none()
    }));
}
