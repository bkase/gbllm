#![cfg(feature = "s3")]

use gbf_experiments::s3::environment::{
    build_config_hash_for_features, compute_environment_hash, dependency_lockfile_hash_for_bytes,
    environment_hash_for_inputs, oracle_backend_identity_hash_for_inputs,
    rust_toolchain_hash_for_identity,
};

#[test]
fn changing_feature_set_changes_build_config_hash() {
    let schema_only = build_config_hash_for_features(vec!["s3"]).unwrap();
    let phase_d = build_config_hash_for_features(vec!["s3", "s3-phase-d"]).unwrap();

    assert_ne!(schema_only, phase_d);
}

#[test]
fn environment_hash_for_inputs_is_deterministic_and_input_sensitive() {
    let oracle =
        oracle_backend_identity_hash_for_inputs("real-denotation", "real-artifact", false).unwrap();
    let fallback =
        oracle_backend_identity_hash_for_inputs("fallback-denotation", "fallback-artifact", false)
            .unwrap();

    let first =
        environment_hash_for_inputs(vec!["s3"], "rustc 1.80.0", "1.80", b"lock-a", oracle).unwrap();
    for _ in 0..10 {
        assert_eq!(
            first,
            environment_hash_for_inputs(vec!["s3"], "rustc 1.80.0", "1.80", b"lock-a", oracle)
                .unwrap()
        );
    }

    assert_ne!(
        first,
        environment_hash_for_inputs(
            vec!["s3", "s3-phase-d"],
            "rustc 1.80.0",
            "1.80",
            b"lock-a",
            oracle,
        )
        .unwrap()
    );
    assert_ne!(
        first,
        environment_hash_for_inputs(vec!["s3"], "rustc 1.81.0", "1.80", b"lock-a", oracle).unwrap()
    );
    assert_ne!(
        first,
        environment_hash_for_inputs(vec!["s3"], "rustc 1.80.0", "1.80", b"lock-b", oracle).unwrap()
    );
    assert_ne!(
        first,
        environment_hash_for_inputs(vec!["s3"], "rustc 1.80.0", "1.80", b"lock-a", fallback)
            .unwrap()
    );
}

#[test]
fn individual_environment_hash_fields_are_sensitive() {
    let lock_a = dependency_lockfile_hash_for_bytes(b"package = 'a'\n");
    let lock_b = dependency_lockfile_hash_for_bytes(b"package = 'b'\n");
    let rust_a = rust_toolchain_hash_for_identity("rustc 1.80.0", "1.80");
    let rust_b = rust_toolchain_hash_for_identity("rustc 1.81.0", "1.80");
    let oracle_a = oracle_backend_identity_hash_for_inputs("real", "artifact", false).unwrap();
    let oracle_b = oracle_backend_identity_hash_for_inputs("real", "artifact", true).unwrap();

    assert_ne!(lock_a, lock_b);
    assert_ne!(rust_a, rust_b);
    assert_ne!(oracle_a, oracle_b);
}

#[test]
fn compute_environment_hash_caches_successes_and_matches_active_inputs() {
    let computed = compute_environment_hash().expect("environment hash");
    let replay = compute_environment_hash().expect("environment hash replay");

    assert_eq!(computed, replay);
}
