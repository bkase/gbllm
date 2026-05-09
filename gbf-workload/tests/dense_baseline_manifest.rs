use std::path::PathBuf;

use gbf_policy::model_profile::ModelSizeProfile;
use gbf_workload::manifest::{
    FfnPathSelection, V0_SUCCESS_ENVELOPE_GATE, WorkloadManifest, WorkloadManifestError,
};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-workload has workspace parent")
        .to_path_buf()
}

fn dense_baseline_manifest_text() -> String {
    std::fs::read_to_string(workspace_root().join("fixtures/workloads/dense_baseline.toml"))
        .expect("dense baseline manifest fixture reads")
}

#[test]
fn dense_baseline_manifest_parses_through_validation() {
    let manifest = WorkloadManifest::from_toml_file(
        workspace_root().join("fixtures/workloads/dense_baseline.toml"),
    )
    .expect("dense baseline manifest parses and validates");

    assert_eq!(manifest.schema, "gbf.workload_manifest.v1");
    assert_eq!(manifest.schema_version, "1.0.0");
    assert_eq!(manifest.id.as_str(), "dense_baseline");
    assert_eq!(
        manifest.corpus.manifest_path,
        "fixtures/corpora/tinystories.toml"
    );
    assert!(manifest.intent.dense_run_end_to_end);
    assert!(manifest.intent.run_goal.contains("dense baseline"));
    assert_eq!(manifest.intent.parity_report_owner, "bd-2zv4");
}

#[test]
fn dense_baseline_selects_registered_dense_profile() {
    let manifest = WorkloadManifest::from_toml_str(&dense_baseline_manifest_text())
        .expect("dense baseline manifest validates");

    assert_eq!(manifest.model.profile, ModelSizeProfile::Toy0);
    assert_eq!(manifest.model.ffn_path, FfnPathSelection::Dense);
    assert_eq!(manifest.model.profile.n_experts(), 0);
    assert_eq!(
        manifest.model.profile.d_model(),
        ModelSizeProfile::TOY0_D_MODEL
    );
    assert_eq!(manifest.model.profile.d_ff(), ModelSizeProfile::TOY0_D_FF);
}

#[test]
fn dense_baseline_references_v0_success_envelope_gate() {
    let manifest = WorkloadManifest::from_toml_str(&dense_baseline_manifest_text())
        .expect("dense baseline manifest validates");

    assert_eq!(
        manifest.acceptance.conformance_gate.envelope,
        V0_SUCCESS_ENVELOPE_GATE
    );
    assert_eq!(
        manifest.acceptance.conformance_gate.report_ref,
        "conformance.json"
    );
    assert_eq!(manifest.prompts.min_generated_chars, 128);
    assert!(manifest.execution.denotational);
    assert!(manifest.execution.artifact);
    assert!(manifest.execution.schedule);
    assert!(manifest.execution.harness);
}

#[test]
fn manifest_rejects_unknown_fields() {
    let mut text = dense_baseline_manifest_text();
    text.push_str("\nunknown_field = true\n");

    assert!(matches!(
        WorkloadManifest::from_toml_str(&text),
        Err(WorkloadManifestError::Toml(_))
    ));
}

#[test]
fn manifest_rejects_wrong_schema_and_gate() {
    let wrong_schema = dense_baseline_manifest_text()
        .replace("gbf.workload_manifest.v1", "gbf.workload_manifest.v2");
    assert!(matches!(
        WorkloadManifest::from_toml_str(&wrong_schema),
        Err(WorkloadManifestError::SchemaMismatch { .. })
    ));

    let wrong_gate =
        dense_baseline_manifest_text().replace(V0_SUCCESS_ENVELOPE_GATE, "future_full_f13_gate");
    assert!(matches!(
        WorkloadManifest::from_toml_str(&wrong_gate),
        Err(WorkloadManifestError::MissingV0SuccessEnvelope { .. })
    ));
}
