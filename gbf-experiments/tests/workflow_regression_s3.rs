#![cfg(feature = "s3")]

use std::fs;
use std::path::{Path, PathBuf};

const PAIRS: [(&str, &str); 3] = [
    ("s2-pr.yml", "s3-pr.yml"),
    ("s2-nightly.yml", "s3-nightly.yml"),
    ("s2-on-demand.yml", "s3-on-demand.yml"),
];

#[test]
fn s3_workflows_preserve_s2_step_structure() {
    for (s2, s3) in PAIRS {
        let s2_source = read_workflow(s2);
        let s3_source = read_workflow(s3);
        assert_eq!(
            step_fingerprint(&s2_source),
            step_fingerprint(&s3_source),
            "{s3} should preserve {s2} step ordering"
        );
        assert_eq!(
            trigger_fingerprint(&s2_source),
            trigger_fingerprint(&s3_source),
            "{s3} should preserve {s2} trigger class"
        );
        assert_artifact_upload_pattern(&s3_source, s3);
    }
}

#[test]
fn s3_pr_gate_order_extends_s2_inherited_gate_order() {
    let s2 = read_workflow("s2-pr.yml");
    let s3 = read_workflow("s3-pr.yml");
    let inherited_s2 = [
        "s2-preregistration",
        "s2-determinism",
        "s2-isolation",
        "s2-api-drift",
    ];
    let inherited_s3 = [
        "s3-preregistration",
        "s3-determinism",
        "s3-isolation",
        "s3-api-drift",
    ];
    assert_order(&s2, &inherited_s2, "s2-pr.yml");
    assert_order(&s3, &inherited_s3, "s3-pr.yml");
    assert_order(
        &s3,
        &[
            "s3-determinism",
            "s3-full-determinism",
            "s3-isolation",
            "s3-api-drift",
            "s3-oracle-re-run",
            "s3-no-naming-resolution",
            "s3-feature-matrix",
        ],
        "s3-pr.yml",
    );
}

#[test]
fn s3_workflows_keep_s2_forensic_capture_contract() {
    for (_s2, s3) in PAIRS {
        let source = read_workflow(s3);
        assert_contains_all(
            &source,
            s3,
            &[
                "run_capture_log()",
                "run_capture_ndjson()",
                "> >(tee \"artifacts/",
                ".stdout\")",
                ".stderr.log\" >&2)",
                ".ndjson\" >&2)",
                "if: always()",
                "if-no-files-found: warn",
            ],
        );
    }
}

fn step_fingerprint(source: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter_map(|line| {
            line.strip_prefix("- uses: ")
                .map(|value| format!("uses:{}", normalize_domain(value)))
                .or_else(|| {
                    line.strip_prefix("- name: ")
                        .map(|value| format!("name:{}", normalize_domain(value)))
                })
        })
        .collect()
}

fn trigger_fingerprint(source: &str) -> Vec<&'static str> {
    let mut triggers = Vec::new();
    if source.contains("pull_request:") {
        triggers.push("pull_request");
    }
    if source.contains("schedule:") {
        triggers.push("schedule");
    }
    if source.contains("workflow_dispatch:") {
        triggers.push("workflow_dispatch");
    }
    triggers
}

fn assert_artifact_upload_pattern(source: &str, workflow: &str) {
    assert_contains_all(
        source,
        workflow,
        &[
            "actions/upload-artifact@v4",
            "artifacts/s3-",
            "**/*.stdout",
            "**/*.stderr.log",
            "**/*.ndjson",
            "/tmp/s3-*.json",
            "experiments/S3/**",
        ],
    );
}

fn assert_order(source: &str, needles: &[&str], workflow: &str) {
    let mut offset = 0;
    for needle in needles {
        let haystack = &source[offset..];
        let Some(index) = haystack.find(needle) else {
            panic!("{workflow} is missing ordered gate {needle}");
        };
        offset += index + needle.len();
    }
}

fn assert_contains_all(source: &str, workflow: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            source.contains(needle),
            "{workflow} is missing expected reference {needle:?}"
        );
    }
}

fn normalize_domain(value: &str) -> String {
    value
        .replace("S2", "SX")
        .replace("S3", "SX")
        .replace("s2", "sx")
        .replace("s3", "sx")
}

fn read_workflow(name: &str) -> String {
    fs::read_to_string(repo_root().join(".github/workflows").join(name))
        .expect("workflow file reads")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
