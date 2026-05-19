#![cfg(feature = "s3")]

use std::fs;
use std::path::{Path, PathBuf};

use toml::Value;

#[test]
fn s3_workspace_crates_have_exact_workspace_pins() {
    let root = workspace_root();
    let cargo_toml = read_toml(&root.join("Cargo.toml"));
    let dependencies = cargo_toml
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Value::as_table)
        .expect("workspace.dependencies table must exist");

    for (crate_name, path) in [
        ("gbf-artifact", "gbf-artifact"),
        ("gbf-workload", "gbf-workload"),
        ("gbf-oracle", "gbf-oracle"),
    ] {
        let dependency = dependencies
            .get(crate_name)
            .and_then(Value::as_table)
            .unwrap_or_else(|| panic!("missing workspace dependency {crate_name}"));
        assert_eq!(
            dependency.get("path").and_then(Value::as_str),
            Some(path),
            "{crate_name} must point at its workspace crate path"
        );
        assert_eq!(
            dependency.get("version").and_then(Value::as_str),
            Some("=0.1.0"),
            "{crate_name} must use exact = version pinning"
        );
    }
}

#[test]
fn s3_experiments_manifest_uses_workspace_dependencies() {
    let manifest = read_toml(&manifest_path().join("Cargo.toml"));
    let dependencies = manifest
        .get("dependencies")
        .and_then(Value::as_table)
        .expect("gbf-experiments dependencies table must exist");

    for crate_name in ["gbf-artifact", "gbf-workload", "gbf-oracle"] {
        let dependency = dependencies
            .get(crate_name)
            .and_then(Value::as_table)
            .unwrap_or_else(|| panic!("missing gbf-experiments dependency {crate_name}"));
        assert_eq!(
            dependency.get("workspace").and_then(Value::as_bool),
            Some(true),
            "{crate_name} must use the workspace-pinned dependency"
        );
    }
}

#[test]
fn s3_feature_contract_is_registered() {
    let manifest = read_toml(&manifest_path().join("Cargo.toml"));
    let features = manifest
        .get("features")
        .and_then(Value::as_table)
        .expect("gbf-experiments features table must exist");

    assert_feature_values(
        features,
        "s3",
        &["gbf-artifact/s3-schemas", "gbf-workload/s3-schemas"],
    );
    assert_feature_values(
        features,
        "s3-phase-d",
        &["gbf-train/qat", "gbf-train/burn-adapter"],
    );
    assert_feature_values(features, "s3-oracle-real", &["gbf-oracle/s3-real"]);
    assert_feature_values(features, "s3-oracle-fallback", &["gbf-oracle/s3-fallback"]);
    assert_feature_values(
        features,
        "s3-oracle-adversarial",
        &["gbf-oracle/s3-oracle-adversarial"],
    );
    assert_feature_values(features, "qat-ablation", &["gbf-train/qat-ablation"]);
    assert!(
        features
            .get("falsify")
            .and_then(Value::as_array)
            .expect("falsify feature must remain registered")
            .iter()
            .any(|value| value.as_str() == Some("gbf-policy/falsify")),
        "falsify must remain the unified test-only feature"
    );
}

#[test]
fn s3_cli_full_feature_forwards_experiment_surface() {
    let manifest = read_toml(&workspace_root().join("gbf-cli/Cargo.toml"));
    let features = manifest
        .get("features")
        .and_then(Value::as_table)
        .expect("gbf-cli features table must exist");

    assert_feature_values(features, "s3-full", &["s3", "s3-phase-d", "s3-oracle-real"]);
    assert_feature_values(features, "s3", &["gbf-experiments/s3"]);
    assert_feature_values(features, "s3-phase-d", &["gbf-experiments/s3-phase-d"]);
    assert_feature_values(
        features,
        "s3-oracle-real",
        &["gbf-experiments/s3-oracle-real"],
    );
}

fn assert_feature_values(features: &toml::map::Map<String, Value>, name: &str, expected: &[&str]) {
    let actual = features
        .get(name)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("missing feature {name}"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("feature {name} contains a non-string entry"))
        })
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "feature {name} changed");
}

fn read_toml(path: &Path) -> Value {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    toml::from_str::<Value>(&text)
        .unwrap_or_else(|error| panic!("failed to parse {}: {error}", path.display()))
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
