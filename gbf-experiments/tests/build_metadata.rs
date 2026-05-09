use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

use gbf_experiments::s1::build_metadata::{
    BUILD_KIND, FALSIFY_ENABLED, QAT_ACTIVE, build_metadata,
};
use gbf_experiments::s1::run::CheckpointMetadata;
use serde_json::json;

#[cfg(feature = "phase-a")]
const EXPECTED_BUILD_KIND: &str = "phase_a";
#[cfg(feature = "ablation")]
const EXPECTED_BUILD_KIND: &str = "ablation";
#[cfg(feature = "phase-a")]
const EXPECTED_QAT_ACTIVE: bool = true;
#[cfg(feature = "ablation")]
const EXPECTED_QAT_ACTIVE: bool = false;

#[test]
fn build_kind_matches_selected_s1_build() {
    assert_eq!(BUILD_KIND, EXPECTED_BUILD_KIND);
    assert_eq!(QAT_ACTIVE, EXPECTED_QAT_ACTIVE);
    assert!(!FALSIFY_ENABLED, "S1-build-A/B must not enable falsify");

    let metadata = build_metadata();
    assert_eq!(metadata.build_kind, EXPECTED_BUILD_KIND);
    assert_eq!(metadata.qat_active, EXPECTED_QAT_ACTIVE);
    assert!(!metadata.gbf_experiments_sha.is_empty());
    assert!(!metadata.gbf_train_sha.is_empty());
    assert_eq!(
        metadata.gbf_experiments_sha, metadata.gbf_train_sha,
        "S1 metadata records the shared workspace HEAD for both crates"
    );
    assert_eq!(
        CheckpointMetadata::current().build_kind,
        EXPECTED_BUILD_KIND
    );
    assert_eq!(
        CheckpointMetadata::default().build_kind,
        EXPECTED_BUILD_KIND
    );
}

#[test]
fn build_metadata_serializes_qat_active_and_workspace_shas() {
    let metadata = build_metadata();
    let value = serde_json::to_value(metadata).expect("build metadata must serialize");

    assert_eq!(
        value,
        json!({
            "build_kind": EXPECTED_BUILD_KIND,
            "qat_active": EXPECTED_QAT_ACTIVE,
            "gbf_experiments_sha": metadata.gbf_experiments_sha,
            "gbf_train_sha": metadata.gbf_train_sha,
        })
    );
}

#[test]
fn phase_a_and_ablation_runtime_metadata_are_observably_distinct() {
    let phase_a = run_build_metadata_probe("s1-phase-a-build-metadata", &["phase-a"]);
    let ablation = run_build_metadata_probe("s1-ablation-build-metadata", &["ablation"]);

    assert_eq!(phase_a, "phase_a|true");
    assert_eq!(ablation, "ablation|false");
    assert_ne!(
        phase_a, ablation,
        "phase-a and ablation builds must expose distinct runtime metadata"
    );
}

fn run_build_metadata_probe(name: &str, gbf_experiments_features: &[&str]) -> String {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s1-build-metadata-")
        .tempdir()
        .expect("build metadata probe tempdir must be creatable");
    let manifest_dir = tempdir.path();
    let src_dir = manifest_dir.join("src");
    fs::create_dir(&src_dir).expect("build metadata probe src directory must be creatable");
    fs::write(src_dir.join("main.rs"), BUILD_METADATA_PROBE)
        .expect("build metadata probe main.rs must be writable");
    fs::write(
        manifest_dir.join("Cargo.toml"),
        probe_manifest(name, gbf_experiments_features),
    )
    .expect("build metadata probe Cargo.toml must be writable");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .arg("run")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", cargo_target_dir())
        .output()
        .expect("build metadata probe cargo run must execute");

    assert!(
        output.status.success(),
        "build metadata probe failed:\n{}",
        command_output(&output)
    );

    utf8_lossy(&output.stdout).trim().to_owned()
}

const BUILD_METADATA_PROBE: &str = r#"
fn main() {
    let metadata = gbf_experiments::s1::build_metadata::build_metadata();
    assert_eq!(
        metadata.qat_active,
        gbf_experiments::s1::build_metadata::QAT_ACTIVE
    );
    println!("{}|{}", metadata.build_kind, metadata.qat_active);
}
"#;

fn probe_manifest(name: &str, gbf_experiments_features: &[&str]) -> String {
    let gbf_experiments = manifest_path();
    let features = gbf_experiments_features
        .iter()
        .map(|feature| format!("{feature:?}"))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        r#"[package]
name = "{name}"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
gbf-experiments = {{ path = "{gbf_experiments}", default-features = false, features = [{features}] }}
"#,
        gbf_experiments = gbf_experiments.display()
    )
}

fn manifest_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    manifest_path()
        .parent()
        .expect("gbf-experiments must live under the workspace root")
        .to_path_buf()
}

fn cargo_target_dir() -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root().join("target"))
}

fn command_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        utf8_lossy(&output.stdout),
        utf8_lossy(&output.stderr)
    )
}

fn utf8_lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
