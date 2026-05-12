use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn s2_full_builds_with_qat_and_burn_adapter() {
    let output = cargo_build_probe("s2-full-feature-probe", &["s2-full"], S2_FULL_PROBE);
    assert!(
        output.status.success(),
        "s2-full probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s2_ablation_builds_without_default_phase_a() {
    let output = cargo_build_probe(
        "s2-ablation-feature-probe",
        &["s2-ablation"],
        S2_ABLATION_PROBE,
    );
    assert!(
        output.status.success(),
        "s2-ablation probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s2_full_and_s2_ablation_trigger_stable_mutex_diagnostic() {
    let output = cargo_build_probe(
        "s2-mutex-feature-probe",
        &["s2-full", "s2-ablation"],
        S2_FULL_PROBE,
    );
    assert!(
        !output.status.success(),
        "s2-full+s2-ablation unexpectedly compiled"
    );

    let combined = command_output(&output);
    assert!(
        combined.contains("S2 feature mutex violated"),
        "S2 mutex probe failed without stable diagnostic:\n{combined}"
    );
}

const S2_FULL_PROBE: &str = r#"
fn main() {
    gbf_experiments::s2::ensure_module_loaded();
    assert_eq!(gbf_experiments::S2_LOG_TARGET, "gbf_experiments::s2");
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
"#;

const S2_ABLATION_PROBE: &str = r#"
fn main() {
    gbf_experiments::s2::ensure_module_loaded();
    assert_eq!(
        gbf_experiments::s1::build_metadata::BUILD_KIND,
        "s1_unselected"
    );
}
"#;

fn cargo_build_probe(name: &str, gbf_experiments_features: &[&str], main_rs: &str) -> Output {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s2-feature-probe-")
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
