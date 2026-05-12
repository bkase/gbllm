#![cfg(feature = "falsify")]

use std::collections::BTreeMap;

use gbf_experiments::s2::falsify::{BrokenKind, FalsificationCaseResult, suite_report};
use gbf_foundation::{Hash256, sha256};

#[test]
fn falsification_s2_suite_hash_is_deterministic_across_replays() {
    let first = suite_report(source_digests(), passing_results()).expect("first report");
    let second = suite_report(source_digests(), passing_results()).expect("second report");

    assert!(first.falsification_s2_passed);
    assert_eq!(
        first.falsification_s2_suite_hash,
        second.falsification_s2_suite_hash
    );
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "falsify_s2__suite_passed",
            serde_json::to_string_pretty(&first).expect("suite report JSON")
        );
    });
}

#[test]
fn modifying_a_test_source_file_changes_the_suite_hash() {
    let baseline = suite_report(source_digests(), passing_results()).expect("baseline report");
    let mut modified = source_digests();
    modified.insert(
        source_path("f1.rs"),
        sha256(b"modified F1 falsification source"),
    );
    let changed = suite_report(modified, passing_results()).expect("changed report");

    assert_ne!(
        baseline.falsification_s2_suite_hash,
        changed.falsification_s2_suite_hash
    );
}

#[test]
fn source_digest_keys_are_workspace_relative_paths() {
    let digests = source_digests();
    assert_eq!(digests.len(), 6);
    for key in digests.keys() {
        assert!(
            key.starts_with("gbf-experiments/tests/falsification_s2/"),
            "source digest key should carry path provenance, got {key}"
        );
        assert!(
            key.ends_with(".rs"),
            "source digest key should identify a Rust test source, got {key}"
        );
    }
}

#[test]
fn suite_failed_one_snapshot_pins_failure_shape() {
    let mut results = passing_results();
    results[2] = FalsificationCaseResult::new(
        BrokenKind::F3DistillTempInverted,
        "config-validator unexpectedly accepted",
        false,
    );
    let report = suite_report(source_digests(), results).expect("failed report");

    assert!(!report.falsification_s2_passed);
    insta::with_settings!({prepend_module_to_snapshot => false}, {
        insta::assert_snapshot!(
            "falsify_s2__suite_failed_one",
            serde_json::to_string_pretty(&report).expect("suite report JSON")
        );
    });
}

fn source_digests() -> BTreeMap<String, Hash256> {
    BTreeMap::from([
        (
            source_path("f1.rs"),
            sha256(include_bytes!("falsification_s2/f1.rs")),
        ),
        (
            source_path("f2.rs"),
            sha256(include_bytes!("falsification_s2/f2.rs")),
        ),
        (
            source_path("f3.rs"),
            sha256(include_bytes!("falsification_s2/f3.rs")),
        ),
        (
            source_path("f4.rs"),
            sha256(include_bytes!("falsification_s2/f4.rs")),
        ),
        (
            source_path("f5.rs"),
            sha256(include_bytes!("falsification_s2/f5.rs")),
        ),
        (
            source_path("f6.rs"),
            sha256(include_bytes!("falsification_s2/f6.rs")),
        ),
    ])
}

fn source_path(file_name: &str) -> String {
    // Keep suite-hash provenance stable and unambiguous even when multiple
    // falsification suites contain `f1.rs`...`f6.rs` helper files.
    format!("gbf-experiments/tests/falsification_s2/{file_name}")
}

fn passing_results() -> Vec<FalsificationCaseResult> {
    [
        BrokenKind::F1PhaseBSkipsTernary,
        BrokenKind::F2PhaseDUnfreezesTeacher,
        BrokenKind::F3DistillTempInverted,
        BrokenKind::F4ThresholdPerWeight,
        BrokenKind::F5ZeroLossShortCircuit,
        BrokenKind::F6LinearStateGradDead,
    ]
    .into_iter()
    .map(|kind| FalsificationCaseResult::new(kind, kind.expected_verdict(), true))
    .collect()
}
