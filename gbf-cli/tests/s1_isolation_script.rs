use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn s1_isolation_script_self_test_covers_o9_failure_diagnostics() {
    let mut command = Command::new("bash");
    command
        .arg(script_path())
        .arg("--self-test")
        .assert()
        .success()
        .stderr(predicate::str::contains("A17 all-identical"))
        .stderr(predicate::str::contains("Rep-7 order-dependence"))
        .stderr(predicate::str::contains("[ISOLATION] self-test PASS"));
}

#[test]
fn s1_isolation_fast_mode_rejects_custom_manifest_without_opt_in() {
    let mut command = Command::new("bash");
    command
        .arg(script_path())
        .arg("--fast")
        .arg("--manifest")
        .arg(workspace_root().join("fixtures/corpora/tinystories.toml"))
        .env_clear()
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains(
            "--fast uses the IntegrationFixture budget",
        ))
        .stderr(predicate::str::contains("--allow-fast-custom-manifest"))
        .stderr(predicate::str::contains(
            "omit --fast for production-budget isolation",
        ));
}

#[test]
fn s1_isolation_script_fast_mode_delegates_to_replay_under_clean_env() {
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
            "S1 isolation PASS mode=fast distinct_final_checkpoint_sha_count=",
        ))
        .stderr(predicate::str::contains("[ISOLATION] running seeds 0..=4"))
        .stderr(predicate::str::contains(
            "[ISOLATION] running [0,1] then [1,0]",
        ))
        .stderr(predicate::str::contains(
            "[ISOLATION] PASS  per-seed hashes order-invariant",
        ));
}

fn script_path() -> PathBuf {
    workspace_root().join("scripts/s1_isolation_check.sh")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}
