#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use clap::Parser;
use gbf_experiments::s3::cli::{S3Cli, S3CliLogging, run};

#[test]
fn fallback_build_kind_without_fallback_feature_returns_structured_error() {
    if cfg!(feature = "s3-oracle-fallback") {
        return;
    }

    let temp = tempfile::tempdir().expect("tempdir");
    let mut cli = S3Cli::parse_from([
        "s3",
        "replay-full",
        "--build-kind",
        "s3_v0_success_fallback_oracle",
        "--output",
        temp.path().join("replay.json").to_str().expect("utf8"),
    ]);
    cli.logging = S3CliLogging::default();

    let error = run(cli).expect_err("fallback build kind should be rejected");
    let message = error.to_string();
    assert!(
        message.contains("s3-v0-success-fallback-oracle"),
        "{message}"
    );
    assert!(message.contains("s3-oracle-fallback"), "{message}");
}
