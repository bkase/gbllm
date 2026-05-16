#![cfg(feature = "falsify")]

mod common;

use std::path::{Path, PathBuf};

use common::tempdir::fresh_isolated_env;
use gbf_experiments::s1::cli::{
    CURRENT_PASS_VERSION, CliBudgetProfile, CliModelProfile, ReplayArgs, S1Cli, S1CliError,
    S1Command, run,
};
use gbf_experiments::s1::schema::RunLog;

#[test]
fn replay_divergence_writes_run_log_without_checkpoint_artifacts() {
    let _env = fresh_isolated_env(&[
        ("BURN_NDARRAY_NUM_THREADS", "1"),
        ("BURN_DETERMINISTIC", "1"),
        ("OMP_NUM_THREADS", "1"),
        ("RAYON_NUM_THREADS", "1"),
    ]);
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("S1");

    let error = run(S1Cli {
        command: S1Command::Replay(ReplayArgs {
            manifest: manifest_path(),
            pass_version: CURRENT_PASS_VERSION,
            seed_list: "0".to_owned(),
            device_profile: "S1CpuDeterministic".to_owned(),
            out_dir: out_dir.clone(),
            budget_profile: CliBudgetProfile::IntegrationFixture,
            model_profile: CliModelProfile::Toy0,
            allow_noncanonical_integration_fixture: true,
            inject_non_finite_loss_at_step: Some(3),
            inject_non_finite_grad_norm_at_step: None,
            zero_gradients: false,
        }),
    })
    .expect_err("diverged replay should return a CLI error");

    assert!(
        matches!(error, S1CliError::RunDiverged { seed: 0 }),
        "{error}"
    );

    let run_log_path = out_dir.join("runs/seed-0/run_log.json");
    assert!(run_log_path.exists(), "missing {run_log_path:?}");
    let run_log: RunLog = serde_json::from_slice(
        &std::fs::read(&run_log_path).unwrap_or_else(|error| panic!("{run_log_path:?}: {error}")),
    )
    .expect("run log parses");
    assert_eq!(run_log.seed, 0);
    assert_eq!(
        run_log.run_log_self_hash,
        run_log.computed_self_hash().expect("run log self hash")
    );

    let checkpoint_dir = out_dir.join("checkpoints/seed-0");
    assert!(
        !checkpoint_dir.join("final.safetensors").exists(),
        "diverged replay must not emit completed checkpoint bytes"
    );
    assert!(
        !checkpoint_dir.join("metadata.json").exists(),
        "diverged replay must not emit checkpoint metadata"
    );
}

fn manifest_path() -> PathBuf {
    repo_root().join("gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml")
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}
