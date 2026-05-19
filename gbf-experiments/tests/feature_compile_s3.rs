#![cfg(feature = "s3")]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn s3_schema_surface_builds_without_phase_d_runtime() {
    let output = cargo_build_probe("s3-schema-feature-probe", &["s3"], S3_SCHEMA_PROBE);
    assert!(
        output.status.success(),
        "s3 schema probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s3_schema_surface_does_not_forward_qat_runtime() {
    let output = cargo_build_probe("s3-schema-no-qat-probe", &["s3"], S3_QAT_ABSENT_PROBE);
    assert!(
        !output.status.success(),
        "s3 schema-only probe unexpectedly exposed gbf-train QAT"
    );
    let combined = command_output(&output);
    assert!(
        combined.contains("could not find `qat`"),
        "s3 schema-only QAT absence probe failed without expected diagnostic:\n{combined}"
    );
}

#[test]
fn s3_schema_surface_does_not_select_oracle_backend() {
    let output = cargo_build_probe(
        "s3-schema-no-oracle-backend-probe",
        &["s3"],
        S3_ORACLE_BACKEND_ABSENT_PROBE,
    );
    assert!(
        !output.status.success(),
        "s3 schema-only probe unexpectedly exposed an oracle backend"
    );
    let combined = command_output(&output);
    assert!(
        combined.contains("S3_REAL_FEATURE_ENABLED"),
        "s3 schema-only oracle absence probe failed without expected diagnostic:\n{combined}"
    );
}

#[test]
fn s3_phase_d_forwards_qat_and_burn_adapter() {
    let output = cargo_build_probe(
        "s3-phase-d-feature-probe",
        &["s3", "s3-phase-d"],
        S3_PHASE_D_PROBE,
    );
    assert!(
        output.status.success(),
        "s3 phase-d probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s3_real_oracle_feature_forwards_to_gbf_oracle() {
    let output = cargo_build_probe(
        "s3-real-oracle-feature-probe",
        &["s3", "s3-phase-d", "s3-oracle-real"],
        S3_REAL_ORACLE_PROBE,
    );
    assert!(
        output.status.success(),
        "s3 real-oracle probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s3_fallback_oracle_feature_forwards_to_gbf_oracle() {
    let output = cargo_build_probe(
        "s3-fallback-oracle-feature-probe",
        &["s3", "s3-phase-d", "s3-oracle-fallback"],
        S3_FALLBACK_ORACLE_PROBE,
    );
    assert!(
        output.status.success(),
        "s3 fallback-oracle probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s3_oracle_backends_trigger_stable_mutex_diagnostic() {
    let output = cargo_build_probe(
        "s3-oracle-mutex-feature-probe",
        &["s3", "s3-oracle-real", "s3-oracle-fallback"],
        S3_SCHEMA_PROBE,
    );
    assert!(
        !output.status.success(),
        "s3 real+fallback oracle probe unexpectedly compiled"
    );

    let combined = command_output(&output);
    assert!(
        combined.contains("s3-oracle-real and s3-oracle-fallback are mutually exclusive")
            || combined
                .contains("gbf-oracle features s3-real and s3-fallback are mutually exclusive"),
        "S3 oracle mutex probe failed without stable diagnostic:\n{combined}"
    );
}

const S3_SCHEMA_PROBE: &str = r#"
fn main() {
    gbf_experiments::s3::ensure_module_loaded();
    assert_eq!(gbf_experiments::S3_LOG_TARGET, "gbf_experiments::s3");
    assert!(gbf_artifact::S3_SCHEMAS_FEATURE_ENABLED);
    assert!(gbf_workload::S3_SCHEMAS_FEATURE_ENABLED);
}
"#;

const S3_PHASE_D_PROBE: &str = r#"
fn main() {
    gbf_experiments::s3::ensure_module_loaded();
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
"#;

const S3_QAT_ABSENT_PROBE: &str = r#"
fn main() {
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
"#;

const S3_REAL_ORACLE_PROBE: &str = r#"
fn main() {
    gbf_experiments::s3::ensure_module_loaded();
    assert!(gbf_oracle::S3_REAL_FEATURE_ENABLED);
}
"#;

const S3_FALLBACK_ORACLE_PROBE: &str = r#"
fn main() {
    gbf_experiments::s3::ensure_module_loaded();
    assert!(gbf_oracle::S3_FALLBACK_FEATURE_ENABLED);
}
"#;

const S3_ORACLE_BACKEND_ABSENT_PROBE: &str = r#"
fn main() {
    assert!(gbf_oracle::S3_REAL_FEATURE_ENABLED);
}
"#;

fn cargo_build_probe(name: &str, gbf_experiments_features: &[&str], main_rs: &str) -> Output {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s3-feature-probe-")
        .tempdir()
        .expect("feature probe tempdir must be creatable");
    let manifest_dir = tempdir.path();
    let src_dir = manifest_dir.join("src");
    fs::create_dir(&src_dir).expect("feature probe src directory must be creatable");
    fs::write(src_dir.join("main.rs"), main_rs).expect("feature probe main.rs must be writable");
    fs::write(
        manifest_dir.join("Cargo.toml"),
        probe_manifest(name, gbf_experiments_features),
    )
    .expect("feature probe Cargo.toml must be writable");

    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    Command::new(cargo)
        .arg("build")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", cargo_target_dir())
        .output()
        .expect("feature probe cargo build must run")
}

fn probe_manifest(name: &str, gbf_experiments_features: &[&str]) -> String {
    let gbf_experiments = manifest_path();
    let root = workspace_root();
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
gbf-artifact = {{ path = "{gbf_artifact}", default-features = false }}
gbf-experiments = {{ path = "{gbf_experiments}", default-features = false, features = [{features}] }}
gbf-oracle = {{ path = "{gbf_oracle}", default-features = false }}
gbf-train = {{ path = "{gbf_train}", default-features = false }}
gbf-workload = {{ path = "{gbf_workload}", default-features = false }}
"#,
        gbf_artifact = root.join("gbf-artifact").display(),
        gbf_experiments = gbf_experiments.display(),
        gbf_oracle = root.join("gbf-oracle").display(),
        gbf_train = root.join("gbf-train").display(),
        gbf_workload = root.join("gbf-workload").display()
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
