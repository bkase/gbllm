#![cfg(feature = "falsify")]

use std::collections::BTreeMap;

use gbf_experiments::s3::falsify::{
    FALSIFICATION_S3_SUITE_HASH, expected_passing_results, suite_report,
};
use gbf_foundation::{Hash256, sha256};

#[test]
fn falsification_s3_suite_hash_matches_pinned_snapshot() {
    let report =
        suite_report(source_digests(), expected_passing_results()).expect("suite report hashes");
    assert_eq!(
        report.falsification_s3_suite_hash.to_string(),
        FALSIFICATION_S3_SUITE_HASH
    );
    assert!(report.falsification_s3_passed);
}

#[test]
fn falsification_s3_suite_hash_is_deterministic() {
    let first =
        suite_report(source_digests(), expected_passing_results()).expect("first suite report");
    let second =
        suite_report(source_digests(), expected_passing_results()).expect("second suite report");
    assert_eq!(
        first.falsification_s3_suite_hash,
        second.falsification_s3_suite_hash
    );
}

#[test]
fn falsification_s3_suite_hash_changes_when_substitute_source_changes() {
    let baseline =
        suite_report(source_digests(), expected_passing_results()).expect("baseline report");
    let mut changed = source_digests();
    let mut f8_bytes = include_bytes!("falsification_s3/f8.rs").to_vec();
    f8_bytes.push(b'\n');
    changed.insert(
        "gbf-experiments/tests/falsification_s3/f8.rs".to_owned(),
        sha256(f8_bytes),
    );
    let changed = suite_report(changed, expected_passing_results()).expect("changed report");
    assert_ne!(
        baseline.falsification_s3_suite_hash,
        changed.falsification_s3_suite_hash
    );
}

fn source_digests() -> BTreeMap<String, Hash256> {
    BTreeMap::from([
        (
            "gbf-experiments/tests/falsification_s3/f1.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f1.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f2.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f2.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f3.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f3.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f4.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f4.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f5.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f5.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f6.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f6.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f7.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f7.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f8.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f8.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3/f9.rs".to_owned(),
            sha256(include_bytes!("falsification_s3/f9.rs")),
        ),
        (
            "gbf-experiments/tests/falsification_s3.rs".to_owned(),
            sha256(include_bytes!("falsification_s3.rs")),
        ),
    ])
}
