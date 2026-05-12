use gbf_experiments::s2::environment::{
    build_config_hash_for_features, compute_environment_hash, dependency_lockfile_hash_for_bytes,
    environment_hash_for_inputs, rust_toolchain_hash_for_identity,
};

#[test]
fn changing_feature_set_changes_build_config_hash() {
    let full = build_config_hash_for_features(vec!["s2-full"]).unwrap();
    let ablation = build_config_hash_for_features(vec!["s2-ablation"]).unwrap();

    assert_ne!(full, ablation);
}

#[test]
fn changing_lockfile_bytes_changes_dependency_lockfile_hash() {
    let base = dependency_lockfile_hash_for_bytes(b"package = 'a'\n");
    let changed = dependency_lockfile_hash_for_bytes(b"package = 'b'\n");

    assert_ne!(base, changed);
}

#[test]
fn changing_toolchain_identity_changes_rust_toolchain_hash() {
    let base = rust_toolchain_hash_for_identity("rustc 1.80.0", "1.80");
    let changed_rustc = rust_toolchain_hash_for_identity("rustc 1.81.0", "1.80");
    let changed_package = rust_toolchain_hash_for_identity("rustc 1.80.0", "1.81");

    assert_ne!(base, changed_rustc);
    assert_ne!(base, changed_package);
}

#[test]
fn environment_hash_for_inputs_is_uncached_and_input_sensitive() {
    let first =
        environment_hash_for_inputs(vec!["s2-full"], "rustc 1.80.0", "1.80", b"lock-a").unwrap();
    let same =
        environment_hash_for_inputs(vec!["s2-full"], "rustc 1.80.0", "1.80", b"lock-a").unwrap();
    let feature_changed =
        environment_hash_for_inputs(vec!["s2-ablation"], "rustc 1.80.0", "1.80", b"lock-a")
            .unwrap();
    let toolchain_changed =
        environment_hash_for_inputs(vec!["s2-full"], "rustc 1.81.0", "1.80", b"lock-a").unwrap();
    let lock_changed =
        environment_hash_for_inputs(vec!["s2-full"], "rustc 1.80.0", "1.80", b"lock-b").unwrap();

    assert_eq!(first, same);
    assert_ne!(first, feature_changed);
    assert_ne!(first, toolchain_changed);
    assert_ne!(first, lock_changed);
}

#[test]
fn compute_environment_hash_matches_explicit_active_inputs() {
    let computed = compute_environment_hash().expect("environment hash");
    let explicit = environment_hash_for_inputs(
        active_test_features(),
        env!("GBF_RUSTC_VERSION"),
        env!("CARGO_PKG_RUST_VERSION"),
        include_bytes!("../../Cargo.lock"),
    )
    .unwrap();

    assert_eq!(computed, explicit);
}

#[test]
fn build_script_documents_rust_toolchain_hash_rerun_triggers() {
    let build_rs = include_str!("../build.rs");

    assert!(build_rs.contains("GBF_RUSTC_VERSION"));
    assert!(build_rs.contains("cargo:rerun-if-changed=../rust-toolchain.toml"));
    assert!(build_rs.contains("cargo:rerun-if-changed=../rust-toolchain"));
    assert!(build_rs.contains("cargo:rerun-if-env-changed=RUSTC"));
    assert!(build_rs.contains("cargo:rerun-if-env-changed=RUSTC_WRAPPER"));
    assert!(build_rs.contains("cargo:rerun-if-env-changed=RUSTUP_TOOLCHAIN"));
    assert!(build_rs.contains("rustup default"));
}

#[test]
fn compute_environment_hash_caches_only_successes_not_error_strings() {
    let environment_rs = include_str!("../src/s2/environment.rs");

    assert!(environment_rs.contains("OnceLock<S2EnvironmentHash>"));
    assert!(!environment_rs.contains("OnceLock<Result<S2EnvironmentHash, String>>"));
    assert!(!environment_rs.contains("S1SchemaError::Custom(message.clone())"));
}

fn active_test_features() -> Vec<&'static str> {
    let mut features = Vec::new();
    if cfg!(feature = "phase-a") {
        features.push("phase-a");
    }
    if cfg!(feature = "ablation") {
        features.push("ablation");
    }
    if cfg!(feature = "s2-full") {
        features.push("s2-full");
    }
    if cfg!(feature = "s2-ablation") {
        features.push("s2-ablation");
    }
    if cfg!(feature = "falsify") {
        features.push("falsify");
    }
    features
}
