use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const QAT_ONLY_PROBE: &str = include_str!("trybuild/qat_only.rs");
const QAT_ABLATION_ONLY_PROBE: &str = include_str!("trybuild/qat_ablation_only.rs");
const QAT_AND_ABLATION_PROBE: &str = include_str!("trybuild/qat_and_qat_ablation_both_enabled.rs");

#[test]
fn phase_a_forwards_qat_and_burn_adapter_to_gbf_train() {
    let output = cargo_check_probe("qat-forwarding", &["phase-a"], QAT_ONLY_PROBE);
    assert!(
        output.status.success(),
        "phase-a forwarding probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn ablation_feature_compiles_without_default_phase_a() {
    let output = cargo_check_probe("qat-ablation-only", &["ablation"], QAT_ABLATION_ONLY_PROBE);
    assert!(
        output.status.success(),
        "ablation-only probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn phase_a_and_ablation_features_trigger_gbf_train_mutex() {
    let output = cargo_check_probe(
        "qat-and-ablation-mutex",
        &["phase-a", "ablation"],
        QAT_AND_ABLATION_PROBE,
    );
    assert!(
        !output.status.success(),
        "phase-a+ablation probe unexpectedly compiled; gbf-train mutex or gbf-experiments feature forwarding may be broken"
    );

    let combined = command_output(&output);
    assert!(
        combined.contains("qat and qat-ablation are mutually exclusive"),
        "mutex probe failed without the gbf-train diagnostic:\n{combined}"
    );
}

fn cargo_check_probe(name: &str, gbf_experiments_features: &[&str], main_rs: &str) -> Output {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s1-feature-probe-")
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
        .arg("check")
        .arg("--quiet")
        .arg("--manifest-path")
        .arg(manifest_dir.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", cargo_target_dir())
        .output()
        .expect("feature probe cargo check must run")
}

fn probe_manifest(name: &str, gbf_experiments_features: &[&str]) -> String {
    let gbf_experiments = manifest_path();
    let gbf_train = workspace_root().join("gbf-train");
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
gbf-train = {{ path = "{gbf_train}", default-features = false }}
"#,
        gbf_experiments = gbf_experiments.display(),
        gbf_train = gbf_train.display()
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
