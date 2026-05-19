#![cfg(feature = "s3")]

use std::path::Path;
use std::process::Command;

#[test]
fn falsify_feature_is_rejected_for_release_non_test_builds() {
    let target_dir = tempfile::tempdir().expect("temporary cargo target dir");
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments lives under workspace root");
    let output = Command::new(env!("CARGO"))
        .current_dir(workspace_root)
        .env("CARGO_TARGET_DIR", target_dir.path())
        .args([
            "check",
            "-p",
            "gbf-experiments",
            "--no-default-features",
            "--features",
            "falsify",
            "--release",
        ])
        .output()
        .expect("cargo check runs");

    assert!(
        !output.status.success(),
        "release cargo check unexpectedly accepted falsify; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("the unified `falsify` feature must only be enabled in test builds"),
        "missing falsify compile_error in stderr:\n{stderr}"
    );
}
