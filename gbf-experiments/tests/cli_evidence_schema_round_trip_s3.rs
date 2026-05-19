#![cfg(all(feature = "s3", feature = "s3-phase-d"))]

use clap::Parser;
use gbf_experiments::s3::cli::evidence_schemas::{
    S3CharsetNormalizeCliEvidence, S3ExportArtifactCliEvidence, S3ExportBundleCliEvidence,
    S3FitBaselineCliEvidence, S3OracleReRunCliEvidence, S3ReplayFullCliEvidence,
    S3ReportCliEvidence, S3VerifyDeterminismCliEvidence, canonical_evidence_bytes,
};
use gbf_experiments::s3::cli::{S3Cli, S3CliError, S3CliLogging, run};
use gbf_experiments::s3::report::{ReportError, ReportValidationError, predictions_section_hash};
use gbf_foundation::Hash256;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[test]
fn s3_cli_evidence_schemas_round_trip_canonically() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = run_cli_json::<S3ReplayFullCliEvidence>(&[
        "s3",
        "replay-full",
        "--output",
        temp.path().join("replay.json").to_str().expect("utf8"),
        "--json",
    ]);
    assert_round_trip(&replay);

    let determinism = run_cli_json::<S3VerifyDeterminismCliEvidence>(&[
        "s3",
        "verify-determinism",
        "--seed-list",
        "0",
        "--output",
        temp.path().join("determinism.json").to_str().expect("utf8"),
        "--json",
    ]);
    assert_round_trip(&determinism);

    let charset = temp.path().join("charset-cli.json");
    run_cli(&[
        "s3",
        "normalize-corpus",
        "--output",
        temp.path().join("charset.json").to_str().expect("utf8"),
        "--evidence-output",
        charset.to_str().expect("utf8"),
    ]);
    assert_round_trip(&read::<S3CharsetNormalizeCliEvidence>(&charset));

    let baseline = temp.path().join("baseline-cli.json");
    run_cli(&[
        "s3",
        "fit-baseline",
        "--output",
        temp.path().join("baseline.json").to_str().expect("utf8"),
        "--evidence-output",
        baseline.to_str().expect("utf8"),
    ]);
    assert_round_trip(&read::<S3FitBaselineCliEvidence>(&baseline));

    let bundle = temp.path().join("bundle-cli.json");
    run_cli(&[
        "s3",
        "export-bundle",
        "--bundle-output",
        temp.path().join("bundle.json").to_str().expect("utf8"),
        "--metadata-output",
        temp.path()
            .join("bundle-metadata.json")
            .to_str()
            .expect("utf8"),
        "--evidence-output",
        bundle.to_str().expect("utf8"),
    ]);
    assert_round_trip(&read::<S3ExportBundleCliEvidence>(&bundle));

    let artifact = temp.path().join("artifact-cli.json");
    run_cli(&[
        "s3",
        "export-artifact",
        "--artifact-output",
        temp.path().join("artifact.bin").to_str().expect("utf8"),
        "--metadata-output",
        temp.path()
            .join("artifact-metadata.json")
            .to_str()
            .expect("utf8"),
        "--evidence-output",
        artifact.to_str().expect("utf8"),
    ]);
    assert_round_trip(&read::<S3ExportArtifactCliEvidence>(&artifact));

    let oracle_re_run = temp.path().join("oracle-re-run-cli.json");
    run_cli(&[
        "s3",
        "oracle-re-run",
        "--output",
        temp.path()
            .join("oracle-re-run.json")
            .to_str()
            .expect("utf8"),
        "--evidence-output",
        oracle_re_run.to_str().expect("utf8"),
    ]);
    let oracle_re_run_evidence = read::<S3OracleReRunCliEvidence>(&oracle_re_run);
    assert!(oracle_re_run_evidence.s1_oracle_re_run_passed);
    assert!(oracle_re_run_evidence.s2_oracle_re_run_passed);
    assert!(oracle_re_run_evidence.metric_count > 0);
    assert_round_trip(&oracle_re_run_evidence);

    let report = temp.path().join("report-cli.json");
    run_cli(&[
        "s3",
        "report",
        "--replay-full",
        temp.path().join("replay.json").to_str().expect("utf8"),
        "--export-bundle",
        bundle.to_str().expect("utf8"),
        "--export-artifact",
        artifact.to_str().expect("utf8"),
        "--oracle-re-run",
        oracle_re_run.to_str().expect("utf8"),
        "--normalize-corpus",
        charset.to_str().expect("utf8"),
        "--fit-baseline",
        baseline.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
        "--evidence-output",
        report.to_str().expect("utf8"),
    ]);
    let report = read::<S3ReportCliEvidence>(&report);
    assert_round_trip(&report);
    assert_consumed(&report, "replay-full");
    assert_consumed(&report, "export-bundle");
    assert_consumed(&report, "export-artifact");
    assert_consumed(&report, "oracle-re-run");
    assert_consumed(&report, "normalize-corpus");
    assert_consumed(&report, "fit-baseline");

    let _hash: Hash256 = report.report_self_hash;
}

#[test]
fn s3_report_rejects_consumed_evidence_schema_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let baseline = temp.path().join("baseline-cli.json");
    run_cli(&[
        "s3",
        "fit-baseline",
        "--output",
        temp.path().join("baseline.json").to_str().expect("utf8"),
        "--evidence-output",
        baseline.to_str().expect("utf8"),
    ]);
    let mut tampered: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&baseline).expect("baseline evidence reads"))
            .expect("baseline evidence parses");
    tampered["schema"] = serde_json::Value::String("s3_wrong_cli.v1".to_owned());
    std::fs::write(
        &baseline,
        serde_json::to_vec(&tampered).expect("tampered evidence encodes"),
    )
    .expect("tampered evidence writes");

    let error = run_cli_result(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--fit-baseline",
        baseline.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
    ])
    .expect_err("report rejects wrong evidence schema");
    assert!(matches!(
        error,
        S3CliError::InvalidEvidenceSchema {
            expected: "s3_fit_baseline_cli.v1",
            ..
        }
    ));
}

#[cfg(any(feature = "s3-oracle-real", feature = "s3-oracle-fallback"))]
#[test]
fn s3_oracle_agreement_cli_evidence_round_trips_canonically() {
    use gbf_experiments::s3::cli::evidence_schemas::S3OracleAgreementCliEvidence;

    let temp = tempfile::tempdir().expect("tempdir");
    let evidence = temp.path().join("agreement-cli.json");
    run_cli(&[
        "s3",
        "oracle-agreement",
        "--output",
        temp.path().join("agreement.json").to_str().expect("utf8"),
        "--evidence-output",
        evidence.to_str().expect("utf8"),
    ]);
    assert_round_trip(&read::<S3OracleAgreementCliEvidence>(&evidence));
}

#[cfg(all(
    any(
        feature = "phase-a",
        feature = "ablation",
        feature = "s2-full",
        feature = "s2-ablation",
        feature = "falsify"
    ),
    any(feature = "s3-oracle-real", feature = "s3-oracle-fallback")
))]
#[test]
fn s3_report_consumes_oracle_agreement_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let agreement = temp.path().join("agreement-cli.json");
    run_cli(&[
        "s3",
        "oracle-agreement",
        "--output",
        temp.path().join("agreement.json").to_str().expect("utf8"),
        "--evidence-output",
        agreement.to_str().expect("utf8"),
    ]);

    let report = temp.path().join("report-cli.json");
    run_cli(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--oracle-agreement",
        agreement.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
        "--evidence-output",
        report.to_str().expect("utf8"),
    ]);

    let report = read::<S3ReportCliEvidence>(&report);
    assert_consumed(&report, "oracle-agreement");
}

#[test]
fn s3_report_uses_explicit_preregistration_pin_for_closure_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let predictions = "Pinned S3 closure predictions from the RFC.";
    let rfc = write_predictions_rfc(temp.path(), predictions);
    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let pin = write_preregistration_pin(
        temp.path(),
        predictions,
        &predictions_commit,
        &first_result_commit,
    );
    let report = temp.path().join("report.md");
    run_cli(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--predictions-rfc",
        rfc.to_str().expect("utf8"),
        "--output",
        report.to_str().expect("utf8"),
    ]);

    let markdown = std::fs::read_to_string(&report).expect("report reads");
    assert!(markdown.contains(predictions));
    let front_matter = report_front_matter(&markdown);
    assert_eq!(front_matter["predictions_commit"], predictions_commit);
    assert_eq!(front_matter["first_result_commit"], first_result_commit);
    assert_eq!(
        front_matter["predictions_section_hash"],
        predictions_section_hash(predictions)
            .expect("predictions hash")
            .to_string()
    );
    assert_eq!(
        front_matter["oracle_owner_beads"]["denotational"],
        "bd-1rcc"
    );
    assert_eq!(front_matter["oracle_owner_beads"]["artifact"], "bd-c4wg");
    assert_eq!(front_matter["conformance_owner_bead"], "bd-35l3");
    assert_eq!(front_matter["e2e_test_owner_bead"], "bd-1wd");
    assert_eq!(front_matter["structured_logging_owner_bead"], "bd-2sd7");
}

#[test]
fn s3_report_rejects_preregistration_pin_rfc_hash_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let rfc = write_predictions_rfc(temp.path(), "Observed RFC predictions.");
    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let pin = write_preregistration_pin(
        temp.path(),
        "Different pinned predictions.",
        &predictions_commit,
        &first_result_commit,
    );

    let error = run_cli_result(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--predictions-rfc",
        rfc.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
    ])
    .expect_err("report rejects mismatched pin/RFC predictions");
    assert!(matches!(
        error,
        S3CliError::HashMismatch {
            field: "predictions_section_hash",
            ..
        }
    ));
}

#[test]
fn s3_report_rejects_explicit_pre_result_pin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let predictions = "Pinned S3 closure predictions from the RFC.";
    let rfc = write_predictions_rfc(temp.path(), predictions);
    let (predictions_commit, _) = current_git_commit_pair();
    let pin = write_preregistration_pin(temp.path(), predictions, &predictions_commit, "");

    let error = run_cli_result(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--predictions-rfc",
        rfc.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
    ])
    .expect_err("report rejects explicit pre-result pin");
    assert!(matches!(error, S3CliError::S1Schema(_)));
}

#[test]
fn s3_report_rejects_preregistration_rfc_without_prediction_section() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let predictions = "Pinned S3 closure predictions from the RFC.";
    let rfc = temp.path().join("missing-predictions.md");
    std::fs::write(&rfc, "# Fixture\n\n## Observed\n\nNo prediction section.\n")
        .expect("RFC fixture writes");
    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let pin = write_preregistration_pin(
        temp.path(),
        predictions,
        &predictions_commit,
        &first_result_commit,
    );

    let error = run_cli_result(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--predictions-rfc",
        rfc.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
    ])
    .expect_err("report rejects RFC without pre-registered predictions");
    assert!(matches!(
        error,
        S3CliError::MissingPreregistrationSection { .. }
    ));
}

#[test]
fn s3_report_rejects_equal_prediction_and_result_commits_from_pin() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let predictions = "Pinned S3 closure predictions from the RFC.";
    let rfc = write_predictions_rfc(temp.path(), predictions);
    let (_, first_result_commit) = current_git_commit_pair();
    let pin = write_preregistration_pin(
        temp.path(),
        predictions,
        &first_result_commit,
        &first_result_commit,
    );

    let error = run_cli_result(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--predictions-rfc",
        rfc.to_str().expect("utf8"),
        "--output",
        temp.path().join("report.md").to_str().expect("utf8"),
    ])
    .expect_err("report rejects equal predictions/result commits");
    assert!(matches!(
        error,
        S3CliError::Report(ReportError::Validation(
            ReportValidationError::PredictionsCommitEqualsFirstResult { .. }
        ))
    ));
}

#[test]
fn s3_report_default_predictions_rfc_matches_registered_pin_hash() {
    let temp = tempfile::tempdir().expect("tempdir");
    let replay = temp.path().join("replay.json");
    run_cli(&[
        "s3",
        "replay-full",
        "--output",
        replay.to_str().expect("utf8"),
    ]);

    let pin_hash = read_preregistration_pin_field("predictions_section_hash");
    let (predictions_commit, first_result_commit) = current_git_commit_pair();
    let pin = write_preregistration_pin_with_hash(
        temp.path(),
        &pin_hash,
        &predictions_commit,
        &first_result_commit,
    );
    let report = temp.path().join("report.md");
    run_cli(&[
        "s3",
        "report",
        "--replay-full",
        replay.to_str().expect("utf8"),
        "--preregistration-pin",
        pin.to_str().expect("utf8"),
        "--output",
        report.to_str().expect("utf8"),
    ]);

    let markdown = std::fs::read_to_string(&report).expect("report reads");
    let front_matter = report_front_matter(&markdown);
    assert_eq!(front_matter["predictions_section_hash"], pin_hash);
}

fn run_cli(args: &[&str]) {
    run_cli_result(args).expect("S3 CLI command succeeds");
}

fn run_cli_result(args: &[&str]) -> Result<(), S3CliError> {
    let mut cli = S3Cli::parse_from(args);
    cli.logging = S3CliLogging::default();
    run(cli)
}

fn run_cli_json<T>(args: &[&str]) -> T
where
    T: DeserializeOwned,
{
    run_cli(args);
    let output = args
        .windows(2)
        .find_map(|window| (window[0] == "--output").then_some(window[1]))
        .expect("test command carries --output");
    read(output)
}

fn read<T>(path: impl AsRef<std::path::Path>) -> T
where
    T: DeserializeOwned,
{
    serde_json::from_slice(&std::fs::read(path).expect("evidence reads")).expect("evidence parses")
}

fn assert_round_trip<T>(value: &T)
where
    T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let bytes = canonical_evidence_bytes(value).expect("canonicalizes");
    let decoded: T = serde_json::from_slice(&bytes).expect("decodes");
    let decoded_bytes = canonical_evidence_bytes(&decoded).expect("decoded canonicalizes");

    assert_eq!(&decoded, value);
    assert_eq!(decoded_bytes, bytes);
}

fn assert_consumed(report: &S3ReportCliEvidence, evidence_kind: &str) {
    assert!(
        report
            .consumed_evidence
            .iter()
            .any(|row| row.evidence_kind == evidence_kind),
        "missing consumed evidence kind {evidence_kind}: {:#?}",
        report.consumed_evidence
    );
}

fn write_predictions_rfc(dir: &std::path::Path, predictions: &str) -> std::path::PathBuf {
    let path = dir.join("F-S3-fixture.md");
    std::fs::write(
        &path,
        format!(
            "# Fixture\n\n## Pre-registered predictions\n\n{predictions}\n\n## Observed\n\nPending.\n"
        ),
    )
    .expect("RFC fixture writes");
    path
}

fn write_preregistration_pin(
    dir: &std::path::Path,
    predictions: &str,
    predictions_commit: &str,
    first_result_commit: &str,
) -> std::path::PathBuf {
    write_preregistration_pin_with_hash(
        dir,
        &predictions_section_hash(predictions)
            .expect("predictions hash")
            .to_string(),
        predictions_commit,
        first_result_commit,
    )
}

fn write_preregistration_pin_with_hash(
    dir: &std::path::Path,
    predictions_section_hash: &str,
    predictions_commit: &str,
    first_result_commit: &str,
) -> std::path::PathBuf {
    let path = dir.join("preregistration.toml");
    std::fs::write(
        &path,
        format!(
            "schema = \"s3_preregistration.v1\"\n\
             predictions_commit = \"{predictions_commit}\"\n\
             predictions_section_hash = \"{}\"\n\
             pass_version_S3 = \"fixture\"\n\
             rfc_revision = \"{predictions_commit}\"\n\
             first_result_commit = \"{first_result_commit}\"\n",
            predictions_section_hash,
        ),
    )
    .expect("pin fixture writes");
    path
}

fn current_git_commit_pair() -> (String, String) {
    let output = std::process::Command::new("git")
        .args(["rev-list", "--max-count=2", "HEAD"])
        .current_dir(workspace_root())
        .output()
        .expect("git rev-list runs");
    assert!(
        output.status.success(),
        "git rev-list failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let commits = String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert!(
        commits.len() >= 2,
        "S3 report tests require HEAD and a parent"
    );
    (commits[1].clone(), commits[0].clone())
}

fn report_front_matter(markdown: &str) -> Value {
    let front_matter = markdown
        .strip_prefix("---\n")
        .expect("front matter opens")
        .split_once("\n---\n")
        .expect("front matter closes")
        .0;
    serde_json::from_str(front_matter).expect("front matter parses")
}

fn read_preregistration_pin_field(field: &str) -> String {
    let text =
        std::fs::read_to_string(workspace_root().join("experiments/S3/preregistration.toml"))
            .expect("S3 preregistration pin reads");
    for line in text.lines() {
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == field {
            return serde_json::from_str(raw.trim()).expect("pin string value parses");
        }
    }
    panic!("missing {field} in S3 preregistration pin")
}

fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .to_path_buf()
}
