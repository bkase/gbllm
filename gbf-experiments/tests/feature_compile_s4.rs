use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

#[test]
fn s4_schema_surface_builds_without_qat_runtime() {
    let output = cargo_build_probe("s4-schema-feature-probe", &["s4"], S4_SCHEMA_PROBE);
    assert!(
        output.status.success(),
        "s4 schema probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s4_schema_surface_does_not_forward_qat_runtime() {
    let output = cargo_build_probe("s4-schema-no-qat-probe", &["s4"], S4_QAT_ABSENT_PROBE);
    assert!(
        !output.status.success(),
        "s4 schema-only probe unexpectedly exposed gbf-train QAT"
    );
    let combined = command_output(&output);
    assert!(
        combined.contains("could not find `qat`"),
        "s4 schema-only QAT absence probe failed without expected diagnostic:\n{combined}"
    );
}

#[test]
fn s4_full_forwards_qat_and_burn_adapter() {
    let output = cargo_build_probe("s4-full-feature-probe", &["s4-full"], S4_FULL_PROBE);
    assert!(
        output.status.success(),
        "s4-full probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s4_falsify_exposes_test_only_module() {
    let output = cargo_build_probe(
        "s4-falsify-feature-probe",
        &["s4-falsify"],
        S4_FALSIFY_PROBE,
    );
    assert!(
        output.status.success(),
        "s4-falsify probe failed:\n{}",
        command_output(&output)
    );
}

#[test]
fn s4_full_and_s4_falsify_trigger_stable_mutex_diagnostic() {
    let output = cargo_build_probe(
        "s4-full-falsify-mutex-probe",
        &["s4-full", "s4-falsify"],
        S4_SCHEMA_PROBE,
    );
    assert!(
        !output.status.success(),
        "s4-full+s4-falsify unexpectedly compiled"
    );

    let combined = command_output(&output);
    assert!(
        combined
            .contains("S4 feature mutex violated: s4-full and s4-falsify are mutually exclusive"),
        "S4 mutex probe failed without stable diagnostic:\n{combined}"
    );
}

const S4_SCHEMA_PROBE: &str = r#"
fn main() {
    gbf_experiments::s4::ensure_module_loaded();
    assert_eq!(gbf_experiments::S4_LOG_TARGET, "gbf_experiments::s4");
    assert!(gbf_artifact::S3_SCHEMAS_FEATURE_ENABLED);
    assert!(gbf_artifact::S4_SCHEMAS_FEATURE_ENABLED);
    assert!(gbf_workload::S3_SCHEMAS_FEATURE_ENABLED);
    assert_eq!(gbf_experiments::s4::MODULE_SURFACE_COUNT, 18);
    assert_eq!(gbf_experiments::s4::TYPE_COUNT, 13);
    assert_eq!(gbf_experiments::s4::schema::S4BuildKind::ALL.len(), 3);
    assert_eq!(
        gbf_experiments::s4::schema::S4BuildKind::phase_d_continuation.as_str(),
        "phase_d_continuation"
    );
    assert_eq!(
        serde_json::to_string(&gbf_experiments::s4::schema::S4BuildKind::phase_d_continuation)
            .unwrap(),
        "\"phase_d_continuation\""
    );
    assert_eq!(
        serde_json::from_str::<gbf_experiments::s4::schema::S4BuildKind>(
            "\"phase_d_continuation\""
        )
        .unwrap(),
        gbf_experiments::s4::schema::S4BuildKind::phase_d_continuation
    );
    assert_eq!(gbf_experiments::s4::schema::S4Outcome::ALL.len(), 10);
    assert_eq!(
        gbf_experiments::s4::rng::s4_stream_domains(),
        ["s4-init-init", "s4-init-batch", "s4-init-shuffle"]
    );
    assert!(
        gbf_experiments::s4::rng::S4_RNG_STREAM_DEFINITION_V1
            .contains("streams=s4-init-init,s4-init-batch,s4-init-shuffle")
    );
    let bundle = gbf_experiments::s4::schema::S4VerifierBundle::closure_candidate();
    assert_eq!(bundle.completions.len(), 5);
    assert_eq!(
        gbf_experiments::s4::verifier::dispatch_s4_outcome(&bundle),
        gbf_experiments::s4::schema::S4Outcome::PassClean
    );
    let _streams = gbf_experiments::s4::rng::S4RngStreams::new(0);
    let train_config = gbf_experiments::s4::schema::S4TrainConfig::pinned();
    assert_eq!(train_config.optimizer_steps, 20_000);
    assert_eq!(train_config.eval_every_steps, 2_000);
    assert_eq!(train_config.optimizer.lr, 5.0e-4);
    assert_eq!(
        gbf_experiments::s4::run::required_train_loss_count(),
        20_000
    );
    assert_eq!(
        gbf_experiments::s4::run::progress_eval_steps(&train_config)
            .unwrap()
            .len(),
        11
    );
    let phase_schedule =
        gbf_experiments::s4::run::S4PhaseDContinuationSchedule::new(&train_config).unwrap();
    assert_eq!(
        phase_schedule.phase().kind(),
        gbf_train::phase::TrainPhaseKind::FullNumericQat
    );
}
"#;

const S4_FULL_PROBE: &str = r#"
fn main() {
    gbf_experiments::s4::ensure_module_loaded();
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
"#;

const S4_QAT_ABSENT_PROBE: &str = r#"
fn main() {
    let _ = core::any::TypeId::of::<gbf_train::qat::ActFakeQuantBurnQat>();
}
"#;

const S4_FALSIFY_PROBE: &str = r#"
fn main() {
    gbf_experiments::s4::ensure_module_loaded();
    let _ = core::any::TypeId::of::<gbf_experiments::s4::falsify::S4FalsificationSurface>();
    assert_eq!(gbf_experiments::s4::falsify::S4FalsificationCase::ALL.len(), 6);
    assert_eq!(
        gbf_experiments::s4::falsify::S4FalsificationCase::TrainRandomInit.case_id(),
        "F4-broken-S4"
    );
}
"#;

fn cargo_build_probe(name: &str, gbf_experiments_features: &[&str], main_rs: &str) -> Output {
    let tempdir = tempfile::Builder::new()
        .prefix("gbf-s4-feature-probe-")
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
gbf-artifact = {{ path = "{gbf_artifact}", default-features = false }}
gbf-experiments = {{ path = "{gbf_experiments}", default-features = false, features = [{features}] }}
gbf-train = {{ path = "{gbf_train}", default-features = false }}
gbf-workload = {{ path = "{gbf_workload}", default-features = false }}
serde_json = "=1.0.149"
"#,
        gbf_artifact = workspace_root().join("gbf-artifact").display(),
        gbf_experiments = gbf_experiments.display(),
        gbf_train = gbf_train.display(),
        gbf_workload = workspace_root().join("gbf-workload").display()
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
