use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn s1_determinism_script_self_test_covers_byte_diff_diagnostics() {
    let mut command = Command::new("bash");
    command
        .arg(script_path())
        .arg("--self-test")
        .assert()
        .success()
        .stderr(predicate::str::contains("[DETERMINISM] self-test PASS"));
}

#[test]
fn s1_determinism_script_fast_mode_delegates_to_cli_under_clean_env() {
    let mut command = Command::new("bash");
    command
        .arg(script_path())
        .arg("--fast")
        .arg("--gbf-bin")
        .arg(assert_cmd::cargo::cargo_bin("gbf-cli"))
        .env_clear()
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "S1 determinism PASS seed=0 mode=fast",
        ))
        .stderr(predicate::str::contains(
            "[DETERMINISM] starting replay 1 of seed 0",
        ))
        .stderr(predicate::str::contains(
            "[DETERMINISM] starting replay 2 of seed 0",
        ))
        .stderr(predicate::str::contains(
            "[DETERMINISM] PASS  safetensors byte-identical",
        ));
}

#[test]
fn s1_determinism_script_json_failure_propagates_cli_detail() {
    let temp = tempfile::tempdir().expect("tempdir");
    let stub = temp.path().join("gbf-stub");
    std::fs::write(
        &stub,
        "#!/usr/bin/env bash\nprintf 'S1 determinism check failed for seed 0: structure=safetensors_bytes byte_offset=7 expected=0xAA observed=0xBB\\n' >&2\nexit 1\n",
    )
    .expect("write stub");
    make_executable(&stub);

    let assert = Command::new("bash")
        .arg(script_path())
        .arg("--fast")
        .arg("--json")
        .arg("--gbf-bin")
        .arg(&stub)
        .env_clear()
        .assert()
        .failure();
    assert
        .stderr(predicate::str::contains(
            "structure=safetensors_bytes byte_offset=7",
        ))
        .stderr(predicate::str::contains(
            "[DETERMINISM] FAIL  gbf s1 verify-determinism exited with status 1",
        ))
        .stdout(predicate::str::contains("\"deterministic\":false"))
        .stdout(predicate::str::contains("\"failure_detail\""))
        .stdout(predicate::str::contains(
            "structure=safetensors_bytes byte_offset=7",
        ));
}

fn script_path() -> PathBuf {
    workspace_root().join("scripts/s1_determinism_check.sh")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions).expect("set executable");
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) {}
