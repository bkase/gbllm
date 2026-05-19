#![cfg(feature = "s3")]

use clap::CommandFactory;
use gbf_experiments::s3::cli::S3Cli;

#[test]
fn s3_cli_help_lists_every_b23_verb() {
    let help = S3Cli::command().render_help().to_string();

    for verb in [
        "replay-full",
        "replay-fallback",
        "verify-determinism",
        "normalize-corpus",
        "fit-baseline",
        "export-bundle",
        "export-artifact",
        "oracle-agreement",
        "oracle-re-run",
        "report",
    ] {
        assert!(
            help.contains(verb),
            "top-level S3 help missed {verb}:\n{help}"
        );
    }
}

#[test]
fn s3_cli_per_verb_help_exposes_canonical_replay_args() {
    for verb in [
        "replay-full",
        "replay-fallback",
        "verify-determinism",
        "normalize-corpus",
        "fit-baseline",
        "export-bundle",
        "export-artifact",
        "oracle-agreement",
        "oracle-re-run",
        "report",
    ] {
        let mut command = S3Cli::command();
        let subcommand = command
            .find_subcommand_mut(verb)
            .unwrap_or_else(|| panic!("missing S3 subcommand {verb}"));
        let help = subcommand.render_help().to_string();
        for arg in [
            "--manifest",
            "--workload",
            "--chrome-budget",
            "--pass-version",
            "--seed-list",
            "--build-kind",
            "--device-profile",
            "--export-visitor-id",
        ] {
            assert!(help.contains(arg), "{verb} help missed {arg}:\n{help}");
        }
    }
}
