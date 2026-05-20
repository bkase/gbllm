#![cfg(feature = "s4")]

use clap::CommandFactory;
use gbf_experiments::s4::cli::S4Cli;

#[test]
fn s4_cli_help_lists_every_f_s4_23_verb() {
    let help = S4Cli::command().render_help().to_string();

    for verb in [
        "replay-full",
        "replay-fallback",
        "harvest-gutenberg-fixture",
        "build-corpus",
        "fit-baseline-gutenberg",
        "contamination",
        "promote",
        "oracle",
        "score-gutenberg",
        "verify-determinism",
        "normalize-corpus",
        "emit-report",
    ] {
        assert!(
            help.contains(verb),
            "top-level S4 help missed {verb}:\n{help}"
        );
    }
}

#[test]
fn s4_cli_per_verb_help_exposes_common_skeleton_args() {
    for verb in [
        "replay-full",
        "replay-fallback",
        "harvest-gutenberg-fixture",
        "build-corpus",
        "fit-baseline-gutenberg",
        "contamination",
        "promote",
        "oracle",
        "score-gutenberg",
        "verify-determinism",
        "normalize-corpus",
        "emit-report",
    ] {
        let mut command = S4Cli::command();
        let subcommand = command
            .find_subcommand_mut(verb)
            .unwrap_or_else(|| panic!("missing S4 subcommand {verb}"));
        let help = subcommand.render_help().to_string();
        for arg in [
            "--gutenberg-manifest",
            "--pass-version",
            "--seed-list",
            "--build-kind",
            "--device-profile",
            "--output",
        ] {
            assert!(help.contains(arg), "{verb} help missed {arg}:\n{help}");
        }
        assert!(
            help.contains("Examples:") && help.contains(&format!("gbf s4 {verb}")),
            "{verb} help missed a concrete example:\n{help}"
        );
    }
}
