#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use clap::Parser;
use gbf_experiments::s3::cli::{S3Cli, S3CliLogging, run};
use serde_json::Value;

#[test]
fn s3_cli_logging_fields_include_lifecycle_and_stage_taxonomy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let mut cli = S3Cli::parse_from([
        "s3",
        "replay-full",
        "--seed-list",
        "0",
        "--output",
        temp.path().join("replay.json").to_str().expect("utf8"),
    ]);
    cli.logging = S3CliLogging {
        capture_events: Some(events.clone()),
        ..S3CliLogging::default()
    };

    run(cli).expect("replay-full succeeds");

    let events = std::fs::read_to_string(events)
        .expect("events read")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("event JSON"))
        .collect::<Vec<_>>();
    let start = by_name(&events, "s3::cli::start");
    assert_eq!(start["target"], "gbf_experiments::s3::cli");
    assert_eq!(start["fields"]["verb"], "replay-full");
    assert_eq!(start["fields"]["build_kind"], "s3_v0_success_real_oracle");
    assert_eq!(start["fields"]["pass_version_S3"], "0.3.0");
    assert_eq!(start["fields"]["args"]["common"]["seed_list"], "0");
    assert!(start["fields"]["args"]["output"].as_str().is_some());

    let stage_start = by_name(&events, "s3::cli::stage_start");
    assert_eq!(stage_start["fields"]["verb"], "replay-full");
    assert!(stage_start["fields"]["stage_name"].as_str().is_some());
    assert!(stage_start["fields"]["stage_index"].as_u64().is_some());

    let stage_complete = by_name(&events, "s3::cli::stage_complete");
    assert_eq!(stage_complete["fields"]["passed"], true);
    assert!(stage_complete["fields"]["duration_ms"].as_u64().is_some());

    let done = by_name(&events, "s3::cli::done");
    assert_eq!(done["fields"]["exit_code"], 0);
    assert!(done["fields"]["total_duration_ms"].as_u64().is_some());
}

fn by_name<'a>(events: &'a [Value], name: &str) -> &'a Value {
    events
        .iter()
        .find(|event| event["fields"]["event_name"] == name)
        .unwrap_or_else(|| panic!("missing {name}: {events:#?}"))
}
