#![cfg(feature = "s3")]

mod report_s3_support;

use gbf_experiments::s1::schema::S1CanonicalJson;
use gbf_experiments::s3::report::{S3ReportValidator, emit_report, validate_report_validator};
use gbf_experiments::s3::schema::{S3Decision, S3Outcome};

#[test]
fn pass_clean_report_emits_canonical_front_matter_and_required_sections() {
    let report = report_s3_support::pass_clean_report();
    let bytes = emit_report(&report).expect("report emits");
    report_s3_support::write_report_if_requested(&report);
    assert!(bytes.starts_with(b"---\n"));
    assert!(!bytes.contains(&b'\r'), "canonical report must use LF only");
    assert!(
        !bytes.windows(2).any(|window| window == b"\r\n"),
        "canonical report must not contain CRLF"
    );
    let closing_front_matter = bytes
        .windows(5)
        .position(|window| window == b"\n---\n")
        .expect("front matter closes with LF delimiter");
    assert_eq!(
        &bytes[closing_front_matter..closing_front_matter + 5],
        b"\n---\n"
    );
    assert_eq!(bytes[4], b'{');
    assert_eq!(bytes[closing_front_matter - 1], b'}');
    let markdown = String::from_utf8(bytes).expect("markdown is UTF-8");

    assert_eq!(report.front_matter.s3_outcome, S3Outcome::PassClean);
    assert_eq!(report.front_matter.decision, S3Decision::ProceedToS4);
    assert!(markdown.starts_with("---\n{"));
    assert!(markdown.contains(r#""schema":"s3_report.v1""#));
    assert!(markdown.contains("## Pre-registered predictions"));
    assert!(markdown.contains("## Reproducibility statement"));
    let front_matter_bytes =
        S1CanonicalJson::to_vec(&report.front_matter).expect("canonical front matter");
    let front_matter = String::from_utf8(front_matter_bytes).expect("front matter UTF-8");
    assert!(front_matter.contains(r#""report_self_hash":"sha256:"#));
    for validator in [
        S3ReportValidator::Predictions,
        S3ReportValidator::AllSeeds,
        S3ReportValidator::SelfHash,
        S3ReportValidator::AllHypotheses,
        S3ReportValidator::OwnerBeads,
        S3ReportValidator::Decision,
        S3ReportValidator::ClosureArtifacts,
    ] {
        validate_report_validator(&report, validator).expect("validator passes");
    }
}
