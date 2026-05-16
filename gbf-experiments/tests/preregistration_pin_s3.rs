use std::path::{Path, PathBuf};
use std::process::Command;

use gbf_experiments::s1::report::predictions_section_hash;
use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

#[test]
fn preregistration_toml_round_trips_and_hash_replays_are_stable() {
    let pin = read_pin();

    assert_eq!(pin.schema, "s3_preregistration.v1");
    assert_commit_id(&pin.predictions_commit);
    assert_commit_id(&pin.rfc_revision);
    assert!(!pin.pass_version_s3.is_empty());
    assert_eq!(pin.first_result_commit, "");

    let rfc = read_workspace_file("history/rfcs/F-S3-v0-success-tinystories.md");
    let section = rfc_predictions_section(&rfc);
    for _ in 0..10 {
        assert_eq!(
            predictions_section_hash(section)
                .expect("S3 predictions hash computes")
                .to_string(),
            pin.predictions_section_hash.to_string()
        );
    }

    let encoded = toml::to_string(&pin).expect("S3 preregistration TOML encodes");
    let decoded: S3PreregistrationPin =
        toml::from_str(&encoded).expect("S3 preregistration TOML decodes");
    assert_eq!(decoded, pin);
}

#[test]
fn editing_rfc_predictions_changes_hash_and_fails_checker() {
    let pin = read_pin();
    let original = read_workspace_file("history/rfcs/F-S3-v0-success-tinystories.md");
    let edited = original.replacen("Predicted ranges", "Predicted rangez", 1);
    assert_ne!(edited, original, "negative control must edit the RFC body");

    let original_section = rfc_predictions_section(&original);
    let edited_section = rfc_predictions_section(&edited);
    assert_ne!(
        predictions_section_hash(original_section).expect("original hash"),
        predictions_section_hash(edited_section).expect("edited hash"),
        "any predictions-section character edit must change the hash"
    );
    assert_eq!(
        predictions_section_hash(original_section)
            .expect("original hash")
            .to_string(),
        pin.predictions_section_hash.to_string()
    );

    let temp = tempfile::tempdir().expect("tempdir");
    let edited_rfc = temp.path().join("F-S3-edited.md");
    std::fs::write(&edited_rfc, edited).expect("write edited RFC fixture");

    let output = Command::new(workspace_root().join("scripts/s3_preregistration_check.sh"))
        .current_dir(workspace_root())
        .arg("--dry-run")
        .arg("--rfc")
        .arg(&edited_rfc)
        .output()
        .expect("run S3 preregistration checker");

    assert!(
        !output.status.success(),
        "edited RFC should fail prereg checker stdout:\n{}stderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("predictions_section_hash mismatch"),
        "stdout:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct S3PreregistrationPin {
    schema: String,
    predictions_commit: String,
    predictions_section_hash: Hash256,
    #[serde(rename = "pass_version_S3")]
    pass_version_s3: String,
    rfc_revision: String,
    first_result_commit: String,
}

fn read_pin() -> S3PreregistrationPin {
    toml::from_str(&read_workspace_file("experiments/S3/preregistration.toml"))
        .expect("S3 preregistration pin parses")
}

fn read_workspace_file(path: &str) -> String {
    std::fs::read_to_string(workspace_root().join(path)).expect("workspace file reads")
}

fn rfc_predictions_section(markdown: &str) -> &str {
    let marker_pairs = [
        ("## Pre-registered predictions\n\n", "\n## Observed\n"),
        ("  ## Pre-registered predictions\n", "\n\n  ## Observed\n"),
    ];
    for (start_marker, end_marker) in marker_pairs {
        let Some(start) = markdown
            .find(start_marker)
            .map(|offset| offset + start_marker.len())
        else {
            continue;
        };
        let Some(end) = markdown[start..]
            .find(end_marker)
            .map(|offset| start + offset)
        else {
            continue;
        };
        return markdown[start..end].trim();
    }
    panic!("RFC has prereg predictions marker followed by observed marker")
}

fn assert_commit_id(value: &str) {
    assert_eq!(value.len(), 40);
    assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(
        value.bytes().all(|byte| !byte.is_ascii_uppercase()),
        "commit ids must be lowercase"
    );
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments parent is workspace root")
        .to_path_buf()
}
