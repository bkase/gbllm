use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use gbf_policy::s5::{S5FrontierRecommendationReport, s5_frontier_recommendation};
use gbf_policy::{
    H13ShadowFinalByteCostStatus, H15FirstCommitCardinalityVerdict, ShadowCompileSampleExpectation,
    ShadowCompileSampleReal, h13_shadow_final_byte_cost_gap, validate_shr1_shadow_sample,
    verify_h15_first_commit_payload_len,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema: String,
    corpus: String,
    owner_bead: String,
    fixture_scope: String,
    spec_sha: String,
    spec_sha_status: String,
    producer_replay_owner: String,
    files: Vec<ManifestEntry>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    path: String,
    sha256: String,
    intent: String,
    owner_bead: String,
}

#[test]
fn s5_fixture_manifest_is_self_consistent_and_has_no_orphans() {
    let fixture_root = fixture_root();
    let manifest = read_manifest(&fixture_root);
    let mut manifest_paths = BTreeSet::new();

    assert_eq!(manifest.schema, "s5_fixture_manifest.v1");
    assert_eq!(manifest.corpus, "fixtures/s5");
    assert_eq!(manifest.owner_bead, "bd-u4fh");
    assert_eq!(manifest.fixture_scope, "SUBSTRATE_ONLY");
    assert_eq!(manifest.spec_sha, "UNSET_PLACEHOLDER_F_S5_RFC_SHA");
    assert_eq!(manifest.spec_sha_status, "placeholder_unset");
    assert_eq!(manifest.producer_replay_owner, "bd-q3zo");

    for entry in &manifest.files {
        assert!(
            !entry.intent.is_empty() && !entry.owner_bead.is_empty(),
            "manifest entry must explain intent and owner: {}",
            entry.path
        );
        let path = fixture_root.join(&entry.path);
        assert!(path.is_file(), "manifest entry is missing: {}", entry.path);
        assert_eq!(sha256_hex(&path), entry.sha256, "{}", entry.path);
        assert!(
            manifest_paths.insert(entry.path.clone()),
            "duplicate manifest entry: {}",
            entry.path
        );
    }

    for path in fixture_files(&fixture_root) {
        let rel = path.strip_prefix(&fixture_root).unwrap().to_string_lossy();
        assert!(
            manifest_paths.contains(rel.as_ref()),
            "fixture file missing from manifest: {rel}"
        );
    }
}

#[test]
fn s5_frontier_golden_reports_recompute_recommendations() {
    for rel in [
        "frontier/recommendation_a.json",
        "frontier/recommendation_b_l_mt4.json",
        "frontier/recommendation_b_l_fix1.json",
        "frontier/recommendation_tie.json",
    ] {
        let path = fixture_root().join(rel);
        let report: S5FrontierRecommendationReport =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        let recomputed = s5_frontier_recommendation(&report.variant_records).unwrap();

        assert_eq!(
            recomputed.frontier_recommendation, report.frontier_recommendation,
            "{rel}"
        );
        assert_eq!(
            recomputed.frontier_leader_variant, report.frontier_leader_variant,
            "{rel}"
        );
    }
}

#[test]
fn s5_boundary_and_shadow_golden_fixtures_bind_existing_policy_helpers() {
    for (rel, expected_status) in [
        (
            "boundary/h13/delta_1024_pass.toml",
            H13ShadowFinalByteCostStatus::StrictPass,
        ),
        (
            "boundary/h13/delta_1025_warn.toml",
            H13ShadowFinalByteCostStatus::WarningBand,
        ),
        (
            "boundary/h13/delta_2048_warn.toml",
            H13ShadowFinalByteCostStatus::WarningBand,
        ),
        (
            "boundary/h13/delta_2049_refute.toml",
            H13ShadowFinalByteCostStatus::Refuted,
        ),
    ] {
        let value = read_toml(rel);
        let shadow = value["shadow_byte_cost"].as_integer().unwrap() as u64;
        let final_cost = value["final_encoded_rom_byte_cost"].as_integer().unwrap() as u64;
        assert_eq!(
            h13_shadow_final_byte_cost_gap(shadow, final_cost).status,
            expected_status,
            "{rel}"
        );
    }

    let broken: ShadowCompileSampleReal = serde_json::from_slice(
        &fs::read(fixture_root().join("shadow/broken_negative_control.sample.json")).unwrap(),
    )
    .unwrap();
    validate_shr1_shadow_sample(
        &broken,
        ShadowCompileSampleExpectation::BrokenNegativeControl,
    )
    .unwrap();
}

#[test]
fn s5_first_commit_payload_fixtures_cover_h15_cardinality() {
    for (rel, expected_len, expected_verdict) in [
        (
            "first_commit/h15/zero_token_payload.bin",
            0,
            H15FirstCommitCardinalityVerdict::Refuted,
        ),
        (
            "first_commit/h15/single_charset_v1_token.bin",
            1,
            H15FirstCommitCardinalityVerdict::Confirmed,
        ),
        (
            "first_commit/h15/two_charset_v1_tokens.bin",
            2,
            H15FirstCommitCardinalityVerdict::Refuted,
        ),
    ] {
        let len = fs::metadata(fixture_root().join(rel)).unwrap().len() as u32;
        assert_eq!(len, expected_len, "{rel}");
        assert_eq!(
            verify_h15_first_commit_payload_len(len).verdict,
            expected_verdict,
            "{rel}"
        );
    }
}

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/s5")
}

fn read_manifest(fixture_root: &Path) -> Manifest {
    toml::from_str(&fs::read_to_string(fixture_root.join("MANIFEST.toml")).unwrap()).unwrap()
}

fn read_toml(rel: &str) -> toml::Value {
    toml::from_str(&fs::read_to_string(fixture_root().join(rel)).unwrap()).unwrap()
}

fn sha256_hex(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(fs::read(path).unwrap());
    format!("{:x}", hasher.finalize())
}

fn fixture_files(fixture_root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_fixture_files(fixture_root, &mut out);
    out.sort();
    out
}

fn collect_fixture_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_fixture_files(&path, out);
        } else if path.file_name().unwrap() != "MANIFEST.toml" {
            out.push(path);
        }
    }
}
