#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use clap::Parser;
use gbf_experiments::s3::cli::evidence_schemas::{
    S3ReplayFullCliEvidence, S3VerifyDeterminismCliEvidence, canonical_evidence_bytes,
};
use gbf_experiments::s3::cli::{S3Cli, S3CliError, S3CliLogging, run};

#[test]
fn canonical_replay_command_is_byte_stable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = temp.path().join("replay-first.json");
    let second = temp.path().join("replay-second.json");

    run_cli(&[
        "s3",
        "replay-full",
        "--manifest",
        "fixtures/baselines/kn_oracle/manifest.toml",
        "--workload",
        "fixtures/workloads/v0_success.toml",
        "--chrome-budget",
        "8192",
        "--pass-version",
        "0.3.0",
        "--seed-list",
        "0,1,2,3,4",
        "--build-kind",
        "s3_v0_success_real_oracle",
        "--device-profile",
        "S1CpuDeterministic",
        "--export-visitor-id",
        "gbf-train.export_visitor.s3.reference_bundle.v1",
        "--output",
        first.to_str().expect("utf8"),
    ]);
    run_cli(&[
        "s3",
        "replay-full",
        "--manifest",
        "fixtures/baselines/kn_oracle/manifest.toml",
        "--workload",
        "fixtures/workloads/v0_success.toml",
        "--chrome-budget",
        "8192",
        "--pass-version",
        "0.3.0",
        "--seed-list",
        "0,1,2,3,4",
        "--build-kind",
        "s3_v0_success_real_oracle",
        "--device-profile",
        "S1CpuDeterministic",
        "--export-visitor-id",
        "gbf-train.export_visitor.s3.reference_bundle.v1",
        "--output",
        second.to_str().expect("utf8"),
    ]);

    let first_bytes = std::fs::read(&first).expect("first replay reads");
    let second_bytes = std::fs::read(&second).expect("second replay reads");
    assert_eq!(first_bytes, second_bytes);

    let replay: S3ReplayFullCliEvidence =
        serde_json::from_slice(&first_bytes).expect("replay evidence parses");
    assert_eq!(replay.schema, "s3_replay_full_cli.v1");
    assert_eq!(replay.per_seed.len(), 5);
    assert_eq!(
        canonical_evidence_bytes(&replay).expect("replay canonicalizes"),
        first_bytes
    );
}

#[test]
fn verify_determinism_persists_failed_evidence_on_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("determinism-failed.json");
    let mut cli = S3Cli::parse_from([
        "s3",
        "verify-determinism",
        "--seed-list",
        "0",
        "--output",
        output.to_str().expect("utf8"),
        "--force-determinism-mismatch-for-test",
    ]);
    cli.logging = S3CliLogging::default();

    let error = run(cli).expect_err("forced mismatch fails");
    assert!(matches!(error, S3CliError::DeterminismMismatch { .. }));

    let evidence: S3VerifyDeterminismCliEvidence =
        serde_json::from_slice(&std::fs::read(&output).expect("failure evidence reads"))
            .expect("failure evidence parses");
    assert_eq!(evidence.schema, "s3_verify_determinism_cli.v1");
    assert!(!evidence.passed);
    assert_ne!(evidence.first_replay_sha, evidence.second_replay_sha);
}

fn run_cli(args: &[&str]) {
    let mut cli = S3Cli::parse_from(args);
    cli.logging = S3CliLogging::default();
    run(cli).expect("S3 CLI replay succeeds");
}
