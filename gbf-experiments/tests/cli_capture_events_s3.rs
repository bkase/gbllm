#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use clap::Parser;
use gbf_experiments::s3::cli::{S3Cli, S3CliLogging, run};
use serde_json::Value;

#[test]
fn s3_cli_capture_events_records_start_and_done_for_each_verb() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");

    let commands: Vec<(&str, Vec<String>, bool)> = vec![
        (
            "replay-full",
            vec![
                "s3".into(),
                "replay-full".into(),
                "--output".into(),
                replay.display().to_string(),
            ],
            true,
        ),
        (
            "replay-fallback",
            vec![
                "s3".into(),
                "replay-fallback".into(),
                "--output".into(),
                temp.path().join("fallback.json").display().to_string(),
            ],
            cfg!(feature = "s3-oracle-fallback"),
        ),
        (
            "verify-determinism",
            vec![
                "s3".into(),
                "verify-determinism".into(),
                "--seed-list".into(),
                "0".into(),
                "--output".into(),
                temp.path().join("determinism.json").display().to_string(),
            ],
            true,
        ),
        (
            "normalize-corpus",
            vec![
                "s3".into(),
                "normalize-corpus".into(),
                "--output".into(),
                temp.path().join("charset.json").display().to_string(),
            ],
            true,
        ),
        (
            "fit-baseline",
            vec![
                "s3".into(),
                "fit-baseline".into(),
                "--output".into(),
                temp.path().join("baseline.json").display().to_string(),
            ],
            true,
        ),
        (
            "export-bundle",
            vec![
                "s3".into(),
                "export-bundle".into(),
                "--bundle-output".into(),
                temp.path().join("bundle.json").display().to_string(),
                "--metadata-output".into(),
                temp.path()
                    .join("bundle-metadata.json")
                    .display()
                    .to_string(),
            ],
            true,
        ),
        (
            "export-artifact",
            vec![
                "s3".into(),
                "export-artifact".into(),
                "--artifact-output".into(),
                temp.path().join("artifact.bin").display().to_string(),
                "--metadata-output".into(),
                temp.path()
                    .join("artifact-metadata.json")
                    .display()
                    .to_string(),
            ],
            true,
        ),
        (
            "oracle-agreement",
            vec![
                "s3".into(),
                "oracle-agreement".into(),
                "--output".into(),
                temp.path().join("agreement.json").display().to_string(),
            ],
            cfg!(any(
                feature = "s3-oracle-real",
                feature = "s3-oracle-fallback"
            )),
        ),
        (
            "oracle-re-run",
            vec![
                "s3".into(),
                "oracle-re-run".into(),
                "--output".into(),
                temp.path().join("oracle-re-run.json").display().to_string(),
            ],
            true,
        ),
        (
            "report",
            vec![
                "s3".into(),
                "report".into(),
                "--replay-full".into(),
                replay.display().to_string(),
                "--output".into(),
                temp.path().join("report.md").display().to_string(),
            ],
            true,
        ),
    ];

    for (verb, args, should_succeed) in commands {
        let events = temp.path().join(format!("{verb}.ndjson"));
        let result = run_with_events(&args, &events);
        assert_eq!(
            result.is_ok(),
            should_succeed,
            "{verb} success expectation changed: {result:?}"
        );
        let events = read_events(&events);
        assert_event(&events, "s3::cli::start", verb);
        let done = assert_event(&events, "s3::cli::done", verb);
        assert_eq!(
            done["fields"]["exit_code"],
            if should_succeed { 0 } else { 1 }
        );
    }
}

fn run_with_events(args: &[String], events: &std::path::Path) -> Result<(), String> {
    let mut cli = S3Cli::parse_from(args);
    cli.logging = S3CliLogging {
        capture_events: Some(events.to_path_buf()),
        ..S3CliLogging::default()
    };
    run(cli).map_err(|error| error.to_string())
}

fn read_events(path: &std::path::Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("events read")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("event JSON"))
        .collect()
}

fn assert_event<'a>(events: &'a [Value], name: &str, verb: &str) -> &'a Value {
    events
        .iter()
        .find(|event| event["fields"]["event_name"] == name && event["fields"]["verb"] == verb)
        .unwrap_or_else(|| panic!("missing event {name} for {verb}: {events:#?}"))
}
