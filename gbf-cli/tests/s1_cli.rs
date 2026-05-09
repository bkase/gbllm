use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use gbf_artifact::ids::ArtifactPath;
use gbf_artifact::tensor::{
    CanonicalTensor, CanonicalTensorKind, CanonicalTensorLayout, CanonicalTensorPayload,
    CanonicalTensorShape, TensorElementType,
};
use gbf_experiments::s1::ablation::{AblationCheckpoint, compare};
use gbf_experiments::s1::manifest::read_tinystories_manifest;
use gbf_experiments::s1::neg_test::negative_test_report_from_bpcs;
use gbf_experiments::s1::run::{
    CheckpointMetadata as CheckpointWriterMetadata, canonical_checkpoint_bytes,
};
use gbf_experiments::s1::schema::{
    BaselineReport, CheckpointMetadata, CountsSummary, GradNormSummary, OracleReport, RunLog,
    S1BuildKind, S1CanonicalJson, S1Completion, ScoreReport, SmoothingScheme,
};
use gbf_experiments::s1::score::RESET_CONTEXT_CHUNK_SIZE;
use gbf_foundation::{Hash256, SemVer, sha256};
use gbf_policy::model_profile::ModelSizeProfile;
use predicates::prelude::*;

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary")
}

#[test]
fn s1_help_lists_subcommands() {
    let mut command = gbf();
    command.arg("s1").arg("--help");

    command.assert().success().stdout(
        predicate::str::contains("doctor")
            .and(predicate::str::contains("inspect"))
            .and(predicate::str::contains("diff-checkpoints"))
            .and(predicate::str::contains("print-config"))
            .and(predicate::str::contains("replay"))
            .and(predicate::str::contains("fit-baseline"))
            .and(predicate::str::contains("oracle"))
            .and(predicate::str::contains("verify-determinism"))
            .and(predicate::str::contains("score"))
            .and(predicate::str::contains("negative-test"))
            .and(predicate::str::contains("ablation"))
            .and(predicate::str::contains("report")),
    );
}

#[test]
fn s1_subcommand_help_is_available() {
    for subcommand in [
        "doctor",
        "inspect",
        "diff-checkpoints",
        "print-config",
        "replay",
        "fit-baseline",
        "oracle",
        "verify-determinism",
        "score",
        "negative-test",
        "ablation",
        "report",
    ] {
        let mut command = gbf();
        command.args(["s1", subcommand, "--help"]);
        command
            .assert()
            .success()
            .stdout(predicate::str::contains("Usage:"));
    }
}

#[test]
fn doctor_json_passes_on_tiny_fixture_under_deterministic_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "doctor",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--out-dir",
        temp.path().to_str().expect("utf-8 path"),
        "--json",
    ]);

    command.assert().success().stdout(
        predicate::str::contains("\"command\": \"doctor\"")
            .and(predicate::str::contains("\"ok\": true"))
            .and(predicate::str::contains("device_profile_enforce"))
            .and(predicate::str::contains("manifest_train_sha256"))
            .and(predicate::str::contains("burn_version_pin")),
    );
}

#[test]
fn print_config_emits_resolved_s1_contract() {
    let mut command = gbf();
    command.args(["s1", "print-config"]);

    command.assert().success().stdout(
        predicate::str::contains("\"command\": \"print-config\"")
            .and(predicate::str::contains("\"optimizer_steps\": 10000"))
            .and(predicate::str::contains("\"build_kind\""))
            .and(predicate::str::contains("\"device_profile\""))
            .and(predicate::str::contains("\"burn-cpu-lockfile-pinned\"")),
    );
}

#[test]
fn replay_missing_required_flag_names_manifest() {
    let mut command = gbf();
    command.arg("s1").arg("replay");

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("--manifest"));
}

#[test]
fn replay_rejects_invalid_device_profile() {
    let mut command = gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed-list",
        "0",
        "--device-profile",
        "NotS1",
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("S1CpuDeterministic"));
}

#[test]
fn replay_rejects_pass_version_mismatch_before_running() {
    let mut command = gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.0.0",
        "--seed-list",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("Rep-5 pass_version mismatch"));
}

#[test]
fn replay_rejects_seed_outside_s1_range() {
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed-list",
        "99",
        "--device-profile",
        "S1CpuDeterministic",
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("S1-Pre-4"));
}

#[test]
fn replay_smoke_writes_tiny_fixture_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("S1");
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed-list",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
        "--budget-profile",
        "integration-fixture",
        "--allow-noncanonical-integration-fixture",
        "--out-dir",
        out_dir.to_str().expect("utf-8 path"),
    ]);

    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"completion\": \"completed\""));
    assert!(
        out_dir
            .join("checkpoints/seed-0/final.safetensors")
            .exists()
    );
    assert!(out_dir.join("checkpoints/seed-0/metadata.json").exists());
    assert!(out_dir.join("runs/seed-0/run_log.json").exists());
}

#[test]
fn inspect_verifies_replay_metadata_self_hash_and_rejects_tamper() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("S1");
    replay_fixture(&out_dir, "0");
    let metadata = out_dir.join("checkpoints/seed-0/metadata.json");

    let mut inspect_ok = gbf();
    inspect_ok.args(["s1", "inspect", metadata.to_str().expect("utf-8 path")]);
    inspect_ok.assert().success().stdout(
        predicate::str::contains("schema: s1_checkpoint.v1")
            .and(predicate::str::contains("self_hash_ok: true")),
    );

    let run_log = out_dir.join("runs/seed-0/run_log.json");
    let mut inspect_run_log = gbf();
    inspect_run_log.args(["s1", "inspect", run_log.to_str().expect("utf-8 path")]);
    inspect_run_log.assert().success().stdout(
        predicate::str::contains("schema: s1_run_log.v1")
            .and(predicate::str::contains("\"eval_points\""))
            .and(predicate::str::contains("\"final_grad_norms\"")),
    );

    let tampered = temp.path().join("tampered-metadata.json");
    let mut text = fs::read_to_string(&metadata).expect("metadata");
    text = text.replacen("\"final_step\":100", "\"final_step\":101", 1);
    fs::write(&tampered, text).expect("tampered metadata");

    let mut inspect_bad = gbf();
    inspect_bad.args(["s1", "inspect", tampered.to_str().expect("utf-8 path")]);
    inspect_bad
        .assert()
        .failure()
        .stdout(predicate::str::contains("self_hash_ok: false"))
        .stderr(predicate::str::contains("self-hash mismatch"));
}

#[test]
fn diff_checkpoints_reports_equal_and_mismatched_fixture_runs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("S1");
    replay_fixture(&out_dir, "0,1");
    let seed0 = out_dir.join("checkpoints/seed-0/final.safetensors");
    let seed1 = out_dir.join("checkpoints/seed-1/final.safetensors");

    let mut equal = gbf();
    equal.args([
        "s1",
        "diff-checkpoints",
        seed0.to_str().expect("utf-8 path"),
        seed0.to_str().expect("utf-8 path"),
    ]);
    equal
        .assert()
        .success()
        .stdout(predicate::str::contains("\"equal\": true"));

    let mut different = gbf();
    different.args([
        "s1",
        "diff-checkpoints",
        seed0.to_str().expect("utf-8 path"),
        seed1.to_str().expect("utf-8 path"),
    ]);
    different
        .assert()
        .failure()
        .stdout(predicate::str::contains("\"equal\": false"))
        .stdout(predicate::str::contains("\"first_byte_mismatch\""))
        .stderr(predicate::str::contains("diff-checkpoints"));
}

#[test]
fn replay_rejects_integration_fixture_without_noncanonical_opt_in() {
    let temp = tempfile::tempdir().expect("tempdir");
    let out_dir = temp.path().join("experiments/S1");
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed-list",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
        "--budget-profile",
        "integration-fixture",
        "--out-dir",
        out_dir.to_str().expect("utf-8 path"),
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("canonical Production by default"))
        .stderr(predicate::str::contains(
            "--allow-noncanonical-integration-fixture",
        ))
        .stderr(predicate::str::contains("experiments/S1"));
    assert!(
        !out_dir.exists(),
        "rejected noncanonical replay must not create experiments/S1 artifacts"
    );
}

#[test]
fn verify_determinism_smoke_passes_on_tiny_fixture() {
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "verify-determinism",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
        "--budget-profile",
        "integration-fixture",
        "--allow-noncanonical-integration-fixture",
    ]);

    command.assert().success().stdout(
        predicate::str::contains("\"deterministic\": true")
            .and(predicate::str::contains("\"run_log_self_hash\""))
            .and(predicate::str::contains("\"checkpoint_self_hash\"")),
    );
}

#[test]
fn verify_determinism_rejects_integration_fixture_without_noncanonical_opt_in() {
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "verify-determinism",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed",
        "0",
        "--device-profile",
        "S1CpuDeterministic",
        "--budget-profile",
        "integration-fixture",
    ]);

    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("gbf s1 verify-determinism"))
        .stderr(predicate::str::contains("canonical Production by default"))
        .stderr(predicate::str::contains(
            "--allow-noncanonical-integration-fixture",
        ));
}

#[test]
fn helper_no_flag_errors_are_actionable() {
    for (args, detail) in [
        (
            vec!["s1", "oracle"],
            "production oracle artifact emission requires --manifest",
        ),
        (
            vec!["s1", "ablation"],
            "production mode requires --phase-a-checkpoint",
        ),
        (vec!["s1", "report"], "requires --production"),
        (
            vec![
                "s1",
                "score",
                "--manifest",
                manifest_path().to_str().expect("utf-8 path"),
            ],
            "production mode requires --checkpoint",
        ),
        (
            vec![
                "s1",
                "negative-test",
                "--manifest",
                manifest_path().to_str().expect("utf-8 path"),
            ],
            "production mode requires --checkpoint",
        ),
    ] {
        let mut command = gbf();
        command.args(args);
        command
            .assert()
            .failure()
            .stderr(predicate::str::contains(detail));
    }
}

#[test]
fn score_help_clarifies_fixture_checkpoint_sha_is_metadata_only() {
    let mut command = gbf();
    command.args(["s1", "score", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("metadata only")
            .and(predicate::str::contains("--fixture-uniform-scorer")),
    );
}

#[test]
fn helper_subcommands_have_tiny_smoke_surfaces() {
    for args in [
        vec![
            "s1",
            "fit-baseline",
            "--manifest",
            manifest_path().to_str().expect("utf-8 path"),
        ],
        vec![
            "s1",
            "score",
            "--manifest",
            manifest_path().to_str().expect("utf-8 path"),
            "--fixture-uniform-scorer",
        ],
        vec![
            "s1",
            "negative-test",
            "--manifest",
            manifest_path().to_str().expect("utf-8 path"),
            "--fixture-uniform-scorer",
        ],
        vec!["s1", "ablation", "--fixture-self-compare"],
        vec!["s1", "oracle", "--smoke"],
        vec!["s1", "report", "--smoke"],
    ] {
        let mut command = gbf();
        command.args(args);
        command
            .assert()
            .success()
            .stdout(predicate::str::contains("{"));
    }
}

#[test]
fn oracle_with_manifest_emits_s1_oracle_report_artifact() {
    let mut command = gbf();
    command.args([
        "s1",
        "oracle",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--seed",
        "3",
    ]);

    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\": \"s1_oracle.v1\""))
        .stdout(predicate::str::contains("\"metric_oracle_passed\": true"))
        .stdout(predicate::str::contains("\"failed_oracle_ids\": []"))
        .stdout(predicate::str::contains("\"oracle_self_hash\""));
}

#[test]
fn score_and_negative_test_load_production_checkpoint_scorer() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (checkpoint, metadata, checkpoint_sha, _) =
        write_production_checkpoint(temp.path(), 0, S1BuildKind::PhaseA, "production");

    let mut score = gbf();
    score.args([
        "s1",
        "score",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--seed",
        "0",
        "--checkpoint",
        checkpoint.to_str().expect("utf-8 path"),
        "--checkpoint-metadata",
        metadata.to_str().expect("utf-8 path"),
    ]);
    score
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\": \"s1_score.v1\""))
        .stdout(predicate::str::contains(checkpoint_sha.to_string()));

    let mut negative = gbf();
    negative.args([
        "s1",
        "negative-test",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--seed",
        "0",
        "--checkpoint",
        checkpoint.to_str().expect("utf-8 path"),
        "--checkpoint-metadata",
        metadata.to_str().expect("utf-8 path"),
    ]);
    negative
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"schema\": \"s1_negative_test.v1\"",
        ))
        .stdout(predicate::str::contains(checkpoint_sha.to_string()));
}

#[test]
fn production_consumers_reject_integration_fixture_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (checkpoint, metadata, _, _) =
        write_production_checkpoint(temp.path(), 0, S1BuildKind::PhaseA, "integration_fixture");

    let mut command = gbf();
    command.args([
        "s1",
        "score",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--seed",
        "0",
        "--checkpoint",
        checkpoint.to_str().expect("utf-8 path"),
        "--checkpoint-metadata",
        metadata.to_str().expect("utf-8 path"),
    ]);
    command.assert().failure().stderr(predicate::str::contains(
        "require budget_profile=production",
    ));
}

#[test]
fn production_consumers_reject_checkpoint_metadata_payload_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (checkpoint, metadata_path, _, mut metadata) =
        write_production_checkpoint(temp.path(), 0, S1BuildKind::PhaseA, "production");
    metadata.checkpoint_safetensors_sha256 = Hash256::ZERO;
    metadata.checkpoint_self_hash = Hash256::ZERO;
    write_s1_json(
        &metadata_path,
        &metadata.with_computed_self_hash().expect("metadata hash"),
    );

    let mut command = gbf();
    command.args([
        "s1",
        "score",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--seed",
        "0",
        "--checkpoint",
        checkpoint.to_str().expect("utf-8 path"),
        "--checkpoint-metadata",
        metadata_path.to_str().expect("utf-8 path"),
    ]);
    command.assert().failure().stderr(predicate::str::contains(
        "checkpoint_safetensors_sha256 mismatch",
    ));
}

#[test]
fn fixture_scorer_rejects_nonzero_checkpoint_hash() {
    let mut command = gbf();
    command.args([
        "s1",
        "score",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--fixture-uniform-scorer",
        "--checkpoint-sha",
        "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    ]);
    command.assert().failure().stderr(predicate::str::contains(
        "cannot stamp a non-zero --checkpoint-sha",
    ));
}

#[test]
fn ablation_loads_production_checkpoints() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (phase_checkpoint, phase_metadata, _, _) =
        write_production_checkpoint(temp.path(), 0, S1BuildKind::PhaseA, "production");
    let ablation_dir = temp.path().join("ablation");
    let (ablation_checkpoint, ablation_metadata, _, _) =
        write_production_checkpoint(&ablation_dir, 0, S1BuildKind::Ablation, "production");

    let mut command = gbf();
    command.args([
        "s1",
        "ablation",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--phase-a-checkpoint",
        phase_checkpoint.to_str().expect("utf-8 path"),
        "--phase-a-metadata",
        phase_metadata.to_str().expect("utf-8 path"),
        "--ablation-checkpoint",
        ablation_checkpoint.to_str().expect("utf-8 path"),
        "--ablation-metadata",
        ablation_metadata.to_str().expect("utf-8 path"),
    ]);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"schema\": \"s1_ablation.v1\""))
        .stdout(predicate::str::contains("\"phase_a_eq_ablation\": true"));
}

#[test]
fn report_collects_production_artifacts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact_dir = temp.path().join("S1");
    write_production_report_artifacts(&artifact_dir);
    let predictions = temp.path().join("predictions.md");
    fs::write(&predictions, "S1 pre-registered prediction fixture.\n").expect("predictions");

    let mut command = gbf();
    command.args([
        "s1",
        "report",
        "--production",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--artifact-dir",
        artifact_dir.to_str().expect("utf-8 path"),
        "--rfc-revision",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--predictions-commit",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--first-result-commit",
        "cccccccccccccccccccccccccccccccccccccccc",
        "--predictions-section-file",
        predictions.to_str().expect("utf-8 path"),
        "--generated-at",
        "2026-05-09T12:00:00Z",
    ]);
    command
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mode\": \"production\""))
        .stdout(predicate::str::contains("\"output_path\""));
    assert!(artifact_dir.join("S1-report.md").exists());
}

#[test]
fn report_rejects_fixture_score_artifact_masquerading_as_production() {
    let temp = tempfile::tempdir().expect("tempdir");
    let artifact_dir = temp.path().join("S1");
    write_production_report_artifacts(&artifact_dir);
    let manifest = read_tinystories_manifest(manifest_path()).expect("manifest");
    let bad_score = ScoreReport {
        schema: "s1_score.v1".to_owned(),
        seed: 0,
        checkpoint_sha: Hash256::ZERO,
        corpus_val_sha: manifest.val_sha256,
        chunk_size: RESET_CONTEXT_CHUNK_SIZE as u64,
        token_count: 16,
        log2_sum: 128.0,
        bpc: 8.0,
        score_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("score hash");
    write_s1_json(&artifact_dir.join("seed-0/s1_score.v1.json"), &bad_score);
    let predictions = temp.path().join("predictions.md");
    fs::write(&predictions, "S1 pre-registered prediction fixture.\n").expect("predictions");

    let mut command = gbf();
    command.args([
        "s1",
        "report",
        "--production",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--artifact-dir",
        artifact_dir.to_str().expect("utf-8 path"),
        "--rfc-revision",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "--predictions-commit",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--first-result-commit",
        "cccccccccccccccccccccccccccccccccccccccc",
        "--predictions-section-file",
        predictions.to_str().expect("utf-8 path"),
    ]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("score checkpoint_sha mismatch"));
}

fn deterministic_gbf() -> Command {
    let mut command = gbf();
    command
        .env_clear()
        .env("BURN_DETERMINISTIC", "1")
        .env("BURN_NDARRAY_NUM_THREADS", "1")
        .env("OMP_NUM_THREADS", "1")
        .env("RAYON_NUM_THREADS", "1");
    command
}

fn replay_fixture(out_dir: &Path, seed_list: &str) {
    let mut command = deterministic_gbf();
    command.args([
        "s1",
        "replay",
        "--manifest",
        manifest_path().to_str().expect("utf-8 path"),
        "--pass-version",
        "0.1.0",
        "--seed-list",
        seed_list,
        "--device-profile",
        "S1CpuDeterministic",
        "--budget-profile",
        "integration-fixture",
        "--allow-noncanonical-integration-fixture",
        "--out-dir",
        out_dir.to_str().expect("utf-8 path"),
    ]);
    command.assert().success();
}

fn write_production_report_artifacts(artifact_dir: &Path) {
    let manifest = read_tinystories_manifest(manifest_path()).expect("manifest");
    let phase_tensors = production_tensors();
    let (phase_checkpoint, _, phase_checkpoint_sha, phase_metadata) =
        write_production_checkpoint(artifact_dir, 0, S1BuildKind::PhaseA, "production");
    let ablation_dir = artifact_dir.join("ablation");
    let (_, _, ablation_checkpoint_sha, ablation_metadata) =
        write_production_checkpoint(&ablation_dir, 0, S1BuildKind::Ablation, "production");
    let ablation_report = compare(
        AblationCheckpoint {
            metadata: &phase_metadata,
            checkpoint_sha: phase_checkpoint_sha,
            tensors: &phase_tensors,
        },
        AblationCheckpoint {
            metadata: &ablation_metadata,
            checkpoint_sha: ablation_checkpoint_sha,
            tensors: &phase_tensors,
        },
    )
    .expect("ablation report");
    let shuffled_hash = manifest
        .val_shuffle_deadeef_sha256
        .expect("tiny manifest shuffle pin");
    let negative = negative_test_report_from_bpcs(
        0,
        phase_checkpoint_sha,
        manifest.val_sha256,
        shuffled_hash,
        8.0,
        8.2,
    )
    .expect("negative report");
    let baseline = BaselineReport {
        schema: "s1_baseline.v1".to_owned(),
        corpus_train_sha: manifest.train_sha256,
        corpus_val_sha: manifest.val_sha256,
        smoothing: SmoothingScheme {
            alpha: 0.01,
            lambdas: [0.6, 0.3, 0.1],
        },
        bpc_3gram: 10.0,
        bpc_2gram: 10.5,
        bpc_unigram: 11.0,
        counts_summary: CountsSummary {
            train_bytes: 1,
            distinct_unigrams: 1,
            distinct_bigrams: 1,
            distinct_trigrams: 1,
        },
        counts_blob_sha256: Hash256::ZERO,
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()
    .expect("baseline hash");
    let oracle = OracleReport::from_oracle_bools(true, true, true, true, true).expect("oracle");

    write_s1_json(&artifact_dir.join("s1_baseline.v1.json"), &baseline);
    write_s1_json(&artifact_dir.join("s1_oracle.v1.json"), &oracle);
    write_s1_json(
        &artifact_dir.join("seed-0/s1_negative_test.v1.json"),
        &negative,
    );
    write_s1_json(
        &artifact_dir.join("seed-0/s1_ablation.v1.json"),
        &ablation_report,
    );

    for seed in 0..5 {
        let (_, metadata_path, checkpoint_sha, _) =
            write_production_checkpoint(artifact_dir, seed, S1BuildKind::PhaseA, "production");
        let run_log = RunLog {
            schema: "s1_run_log.v1".to_owned(),
            seed,
            train_config_hash: Hash256::ZERO,
            losses: vec![(1, 1.0)],
            eval_points: vec![(0, 8.0), (10_000, 8.0)],
            final_grad_norms: GradNormSummary {
                global_l2: 1.0,
                max_l2: 1.0,
                mean_l2: 1.0,
            },
            run_log_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
        .expect("run log hash");
        let score = ScoreReport {
            schema: "s1_score.v1".to_owned(),
            seed,
            checkpoint_sha,
            corpus_val_sha: manifest.val_sha256,
            chunk_size: RESET_CONTEXT_CHUNK_SIZE as u64,
            token_count: 16,
            log2_sum: 128.0,
            bpc: 8.0,
            score_self_hash: Hash256::ZERO,
        }
        .with_computed_self_hash()
        .expect("score hash");
        write_s1_json(
            &artifact_dir.join(format!("runs/seed-{seed}/run_log.json")),
            &run_log,
        );
        write_s1_json(
            &artifact_dir.join(format!("seed-{seed}/s1_score.v1.json")),
            &score,
        );
        assert!(
            metadata_path.exists(),
            "metadata for seed {seed} should have been written"
        );
    }
    assert!(phase_checkpoint.exists());
}

fn write_production_checkpoint(
    root: &Path,
    seed: u64,
    build_kind: S1BuildKind,
    budget_profile: &str,
) -> (PathBuf, PathBuf, Hash256, CheckpointMetadata) {
    let manifest = read_tinystories_manifest(manifest_path()).expect("manifest");
    let metadata = CheckpointMetadata {
        schema: "s1_checkpoint.v1".to_owned(),
        seed,
        corpus_train_sha: manifest.train_sha256,
        corpus_val_sha: manifest.val_sha256,
        model_config_hash: Hash256::ZERO,
        train_config_hash: Hash256::ZERO,
        build_kind,
        build_config_hash: Hash256::ZERO,
        dependency_lockfile_sha: Hash256::ZERO,
        rust_toolchain_hash: Hash256::ZERO,
        device_profile_hash: Hash256::ZERO,
        rng_stream_def_hash: Hash256::ZERO,
        pass_version: SemVer::new(0, 1, 0),
        budget_profile: budget_profile.to_owned(),
        final_step: 10_000,
        final_train_loss: 1.0,
        completion: S1Completion::Completed,
        checkpoint_safetensors_sha256: Hash256::ZERO,
        checkpoint_self_hash: Hash256::ZERO,
    };
    let tensors = production_tensors();
    let writer_metadata = CheckpointWriterMetadata {
        build_kind: match build_kind {
            S1BuildKind::PhaseA => "phase_a",
            S1BuildKind::Ablation => "ablation",
        },
    };
    let bytes = canonical_checkpoint_bytes(&tensors, &writer_metadata).expect("checkpoint bytes");
    let checkpoint_sha = sha256(&bytes);
    let metadata = CheckpointMetadata {
        checkpoint_safetensors_sha256: checkpoint_sha,
        ..metadata
    }
    .with_computed_self_hash()
    .expect("metadata hash");
    let dir = root.join("checkpoints").join(format!("seed-{seed}"));
    fs::create_dir_all(&dir).expect("checkpoint dir");
    let checkpoint_path = dir.join("final.safetensors");
    let metadata_path = dir.join("metadata.json");
    fs::write(&checkpoint_path, bytes).expect("checkpoint write");
    write_s1_json(&metadata_path, &metadata);
    (checkpoint_path, metadata_path, checkpoint_sha, metadata)
}

fn production_tensors() -> Vec<CanonicalTensor> {
    let d_model = usize::from(ModelSizeProfile::toy0().d_model());
    let d_ff = usize::from(ModelSizeProfile::toy0().d_ff());
    vec![
        production_tensor("toy0.production.embedding_tied.weight", &[256, d_model]),
        production_tensor(
            "toy0.production.linear_state.input_to_state.weight",
            &[d_model, 4],
        ),
        production_tensor(
            "toy0.production.linear_state.state_to_output.weight",
            &[4, d_model],
        ),
        production_tensor("toy0.production.dense_ffn.up.weight", &[d_model, d_ff]),
        production_tensor("toy0.production.dense_ffn.down.weight", &[d_ff, d_model]),
    ]
}

fn production_tensor(name: &str, shape: &[usize]) -> CanonicalTensor {
    let element_count = shape.iter().product();
    CanonicalTensor::new(
        ArtifactPath::new(name).expect("artifact path"),
        CanonicalTensorKind::DenseWeight,
        CanonicalTensorLayout::new(
            CanonicalTensorShape::from_usize_dims(shape).expect("shape"),
            TensorElementType::Float32,
        ),
        CanonicalTensorPayload::F32(vec![0.0; element_count]),
    )
    .expect("canonical tensor")
}

fn write_s1_json<T: serde::Serialize>(path: &Path, value: &T) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("parent dir");
    }
    fs::write(
        path,
        S1CanonicalJson::to_vec(value).expect("canonical json"),
    )
    .expect("json write");
}

fn manifest_path() -> PathBuf {
    workspace_root().join("gbf-experiments/tests/fixtures/tiny_corpus/manifest.toml")
}

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
}
