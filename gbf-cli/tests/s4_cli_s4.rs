#![cfg(feature = "s4")]

use assert_cmd::Command;
use bzip2::write::BzEncoder;
use flate2::write::GzEncoder;
use gbf_artifact::{
    GutenbergCompressionKind, GutenbergDedupPolicy, GutenbergFetchNamespaceKind, GutenbergManifest,
    GutenbergSourceRecord, GutenbergSplit,
};
use gbf_experiments::s4::promote::{
    PromotionGateProduct, S3CheckpointPromotionArtifact, S3OracleAgreementOutcome,
    S3OracleAgreementPromotionArtifact, S3RepetitionCollapseOutcome,
    S3RepetitionCollapsePromotionArtifact, S3V0SuccessPromotionArtifact,
    S4BaselineGutenbergPromotionArtifact, S4ContaminationOutcome, S4ContaminationPromotionArtifact,
    V0SuccessAcceptanceBits, V0SuccessGateOutcome,
};
use gbf_foundation::sha256;
use gbf_foundation::{CanonicalJson, Hash256};
use predicates::prelude::*;
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::{Cursor, Write};
use std::path::Path;
use std::process::Output;
use tar::Header;
use zip::write::FileOptions;

fn gbf() -> Command {
    Command::cargo_bin("gbf-cli").expect("gbf-cli binary")
}

#[test]
fn s4_help_lists_dispatch_verbs() {
    let mut command = gbf();
    command.args(["s4", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("replay-full")
            .and(predicate::str::contains("replay-fallback"))
            .and(predicate::str::contains("harvest-gutenberg-fixture"))
            .and(predicate::str::contains("build-corpus"))
            .and(predicate::str::contains("fit-baseline-gutenberg"))
            .and(predicate::str::contains("contamination"))
            .and(predicate::str::contains("promote"))
            .and(predicate::str::contains("oracle"))
            .and(predicate::str::contains("score-gutenberg"))
            .and(predicate::str::contains("verify-determinism"))
            .and(predicate::str::contains("normalize-corpus"))
            .and(predicate::str::contains("emit-report")),
    );
}

#[test]
fn s4_harvest_help_names_network_permitted_scope() {
    let mut command = gbf();
    command.args(["s4", "harvest-gutenberg-fixture", "--help"]);

    command.assert().success().stdout(
        predicate::str::contains("network-permitted")
            .and(predicate::str::contains("--network-permitted"))
            .and(predicate::str::contains("--fixture-output"))
            .and(predicate::str::contains("--catalog-url")),
    );
}

#[test]
fn s4_cli_dispatches_every_f_s4_23_verb_to_skeleton_evidence() {
    let temp = tempfile::tempdir().expect("tempdir");

    for (verb, owner_bead) in [
        ("replay-full", "bd-10iq"),
        ("replay-fallback", "bd-25pg"),
        ("fit-baseline-gutenberg", "bd-2nca"),
        ("contamination", "bd-2p3n"),
        ("oracle", "bd-3pcy"),
        ("score-gutenberg", "bd-2eun"),
        ("verify-determinism", "bd-u6tn"),
        ("normalize-corpus", "bd-1zd1"),
        ("emit-report", "bd-3f5b"),
    ] {
        let output = temp.path().join(format!("{verb}.json"));
        let mut command = gbf();
        command.args([
            "s4",
            verb,
            "--seed-list",
            "0",
            "--output",
            output.to_str().expect("utf8 output path"),
        ]);

        let output_result = command.output().expect("s4 command runs");
        assert!(
            output_result.status.success(),
            "{verb} failed:\n{}",
            command_output(&output_result)
        );
        let artifact_self_hash = single_stdout_hash(&output_result);
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        assert!(
            stderr.contains("s4_cli_verb_started") && stderr.contains("s4_cli_verb_finalized"),
            "{verb} missed structured stderr lifecycle events:\n{stderr}"
        );

        let evidence: Value =
            serde_json::from_slice(&std::fs::read(&output).expect("dispatch evidence reads"))
                .expect("dispatch evidence parses");
        assert_eq!(evidence["schema"], "s4_cli_dispatch.v1");
        assert_eq!(evidence["artifact_self_hash"], artifact_self_hash);
        assert_eq!(evidence["status"], "skeleton_dispatched");
        assert_eq!(evidence["command"], verb);
        assert_eq!(evidence["owner_bead"], owner_bead);
        assert_eq!(evidence["build_kind"], "phase_d_continuation");
        assert_eq!(evidence["seed_list"], "0");
        assert_eq!(evidence["falsification_cli_verbs_registered"], false);
        assert_eq!(evidence["falsification_verbs_owner_bead"], "bd-fii7");
        assert_eq!(evidence["network_policy"], "disabled");
        assert_eq!(evidence["behavior_deferred"], true);
    }
}

#[test]
fn s4_promote_cli_emits_promotion_gate_artifact() {
    let temp = tempfile::tempdir().expect("tempdir");
    let tinystories_manifest_self_hash = promotion_hash(9);
    let checkpoint = S3CheckpointPromotionArtifact::new("phase_d_resumable", true, true)
        .expect("checkpoint summary builds");
    let manifest = promotion_manifest(0.001);
    let v0_success = S3V0SuccessPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        V0SuccessAcceptanceBits::all_pass(),
        V0SuccessGateOutcome::Pass,
        0.25,
    )
    .expect("v0_success summary builds");
    let oracle = S3OracleAgreementPromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3OracleAgreementOutcome::Agree,
        true,
    )
    .expect("oracle summary builds");
    let contamination = S4ContaminationPromotionArtifact::new(
        tinystories_manifest_self_hash,
        manifest.manifest_self_hash,
        S4ContaminationOutcome::Warn {
            findings: vec!["TS_val_overlaps_GB_val_diagnostic".to_owned()],
        },
    )
    .expect("contamination summary builds");
    let baseline = S4BaselineGutenbergPromotionArtifact::new(
        manifest.manifest_self_hash,
        manifest.train_sha256,
        manifest.val_sha256,
        2.75,
    )
    .expect("baseline summary builds");
    let repetition = S3RepetitionCollapsePromotionArtifact::new(
        checkpoint.checkpoint_self_hash,
        tinystories_manifest_self_hash,
        S3RepetitionCollapseOutcome::Pass,
    )
    .expect("repetition summary builds");

    let checkpoint_path = temp.path().join("checkpoint.json");
    let manifest_path = temp.path().join("gutenberg-manifest.json");
    let v0_path = temp.path().join("v0-success.json");
    let oracle_path = temp.path().join("oracle-agreement.json");
    let contamination_path = temp.path().join("contamination.json");
    let baseline_path = temp.path().join("baseline.json");
    let repetition_path = temp.path().join("repetition.json");
    let output = temp.path().join("promotion-gate.json");

    write_json_artifact(&checkpoint_path, &checkpoint);
    write_json_artifact(&manifest_path, &manifest);
    write_json_artifact(&v0_path, &v0_success);
    write_json_artifact(&oracle_path, &oracle);
    write_json_artifact(&contamination_path, &contamination);
    write_json_artifact(&baseline_path, &baseline);
    write_json_artifact(&repetition_path, &repetition);

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s4",
        "promote",
        "--tinystories-manifest-self-hash",
        &tinystories_manifest_self_hash.to_string(),
        "--c-ts",
        checkpoint_path.to_str().expect("utf8 checkpoint path"),
        "--c-ts-v0success",
        v0_path.to_str().expect("utf8 v0 path"),
        "--c-ts-oracle-agreement",
        oracle_path.to_str().expect("utf8 oracle path"),
        "--gutenberg-manifest",
        manifest_path.to_str().expect("utf8 manifest path"),
        "--contamination-report",
        contamination_path
            .to_str()
            .expect("utf8 contamination path"),
        "--baseline-gutenberg",
        baseline_path.to_str().expect("utf8 baseline path"),
        "--repetition-collapse-check",
        repetition_path.to_str().expect("utf8 repetition path"),
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    let output_result = command.output().expect("s4 promote runs");
    assert!(
        output_result.status.success(),
        "promote failed:\n{}",
        command_output(&output_result)
    );
    let promotion_gate_self_hash = single_stdout_hash(&output_result);
    let product: PromotionGateProduct =
        serde_json::from_slice(&std::fs::read(&output).expect("promotion artifact reads"))
            .expect("promotion artifact parses");
    product
        .validate_canonical_write()
        .expect("promotion gate self-hash validates");
    assert_eq!(
        product.promotion_gate_self_hash.to_string(),
        promotion_gate_self_hash
    );
    assert_eq!(product.schema, "s4_promotion_gate.v1");
    assert_eq!(
        product.contamination_self_hash,
        contamination.contamination_self_hash
    );
    assert_eq!(
        product.input_artifacts.contamination_report.artifact_path,
        contamination_path.to_string_lossy()
    );
    assert!(
        matches!(
            product.outcome,
            gbf_experiments::s4::promote::PromotionGateOutcome::Promoted { .. }
        ),
        "Warn contamination input should promote"
    );
}

#[test]
fn s4_build_corpus_cli_writes_offline_manifest_quality_and_streams() {
    let root = workspace_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest = temp.path().join("gutenberg-manifest.json");
    let train = temp.path().join("gutenberg-train.bin");
    let val = temp.path().join("gutenberg-val.bin");
    let quality = temp.path().join("corpus-quality.json");
    let output = temp.path().join("build-corpus.json");

    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s4",
        "build-corpus",
        "--fixture",
        root.join("fixtures/corpora/gutenberg_smoke.toml")
            .to_str()
            .expect("utf8 fixture path"),
        "--gutenberg-manifest",
        manifest.to_str().expect("utf8 manifest path"),
        "--train-output",
        train.to_str().expect("utf8 train path"),
        "--val-output",
        val.to_str().expect("utf8 val path"),
        "--corpus-quality-output",
        quality.to_str().expect("utf8 quality path"),
        "--tinystories-manifest",
        root.join("fixtures/corpora/tinystories.toml")
            .to_str()
            .expect("utf8 TinyStories path"),
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    let output_result = command.output().expect("s4 build-corpus runs");
    assert!(
        output_result.status.success(),
        "build-corpus failed:\n{}",
        command_output(&output_result)
    );
    let manifest_self_hash = single_stdout_hash(&output_result);
    let evidence: Value =
        serde_json::from_slice(&std::fs::read(&output).expect("build evidence reads"))
            .expect("build evidence parses");
    assert_eq!(evidence["schema"], "s4_build_corpus_cli.v1");
    assert_eq!(evidence["status"], "built");
    assert_eq!(evidence["owner_bead"], "bd-29lv");
    assert_eq!(evidence["network_policy"], "disabled");
    assert_eq!(evidence["behavior_deferred"], false);
    assert_eq!(evidence["manifest_self_hash"], manifest_self_hash);
    assert_eq!(evidence["train_book_count"], 7);
    assert_eq!(evidence["val_book_count"], 1);
    assert_eq!(evidence["drop_count_total"], 0);
    assert_eq!(evidence["unmappable_gate"]["status"], "emitted");
    assert_eq!(evidence["unmappable_gate"]["owner_bead"], "bd-bzx3");
    assert_eq!(
        evidence["unmappable_gate"]["artifact_path"],
        quality.to_str().expect("utf8 quality path")
    );
    assert!(evidence["deferred_owner_beads"]["unmappable_gate"].is_null());
    assert!(manifest.exists());
    assert!(train.exists());
    assert!(val.exists());
    assert!(quality.exists());
}

#[test]
fn s4_harvest_requires_explicit_network_permission() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("harvest.json");

    let mut command = gbf();
    command.args([
        "s4",
        "harvest-gutenberg-fixture",
        "--target-slice",
        "1",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("--network-permitted"));
}

#[test]
fn s4_harvest_writes_hash_pinned_fixture_from_local_rdf_catalog() {
    let temp = tempfile::tempdir().expect("tempdir");
    let corpus_dir = temp.path().join("corpus");
    let plain_path = temp.path().join("plain-10.txt");
    let latin1_path = temp.path().join("latin1-20.txt");
    let gzip_path = temp.path().join("gzip-30.txt.gz");
    let zip_path = temp.path().join("zip-40.zip");
    std::fs::write(&plain_path, b"Plain UTF-8\n").expect("plain source writes");
    std::fs::write(&latin1_path, b"Caf\xe9\n").expect("latin1 source writes");
    std::fs::write(&gzip_path, gzip_bytes(b"Gzip UTF-8\n")).expect("gzip source writes");
    std::fs::write(&zip_path, zip_bytes("text/book40.txt", b"Zip UTF-8\n"))
        .expect("zip source writes");

    let catalog_path = temp.path().join("rdf-files.tar.bz2");
    write_catalog(
        &catalog_path,
        &[
            rdf_entry(10, &file_url(&plain_path), "text/plain; charset=utf-8", 12),
            rdf_entry(
                20,
                &file_url(&latin1_path),
                "text/plain; charset=iso-8859-1",
                5,
            ),
            rdf_entry(
                30,
                &file_url(&gzip_path),
                "application/gzip; charset=utf-8",
                std::fs::metadata(&gzip_path).unwrap().len(),
            ),
            rdf_entry(
                40,
                &file_url(&zip_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&zip_path).unwrap().len(),
            ),
        ],
    );

    let fixture_output = temp.path().join("gutenberg.toml");
    let output = temp.path().join("harvest.json");
    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&catalog_path),
        "--catalog-path",
        catalog_path.to_str().expect("utf8 catalog path"),
        "--catalog-observed-at-utc",
        "2026-05-19T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-18T00:00:00Z",
        "--cache-dir",
        corpus_dir.to_str().expect("utf8 corpus dir"),
        "--fixture-output",
        fixture_output.to_str().expect("utf8 fixture path"),
        "--target-slice",
        "4",
        "--fetch-namespace-kind",
        "local_private_mirror",
        "--fetch-namespace-id",
        "file://local-gutenberg-fixture/",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    let output_result = command.output().expect("s4 harvest runs");
    assert!(
        output_result.status.success(),
        "harvest failed:\n{}",
        command_output(&output_result)
    );
    let artifact_self_hash = single_stdout_hash(&output_result);
    let evidence: Value =
        serde_json::from_slice(&std::fs::read(&output).expect("harvest evidence reads"))
            .expect("harvest evidence parses");
    assert_eq!(evidence["schema"], "s4_gutenberg_harvest.v1");
    assert_eq!(evidence["artifact_self_hash"], artifact_self_hash);
    assert_eq!(evidence["status"], "fixture_harvested");
    assert_eq!(evidence["owner_bead"], "bd-1zd1");
    assert_eq!(evidence["network_policy"], "permitted");
    assert_eq!(evidence["behavior_deferred"], false);
    assert_eq!(evidence["harvest"]["book_count"], 4);
    assert_eq!(evidence["harvest"]["source_count"], 4);

    let fixture_text = std::fs::read_to_string(&fixture_output).expect("fixture reads");
    let fixture: toml::Value = toml::from_str(&fixture_text).expect("fixture parses");
    let catalog_sha256 = sha256(std::fs::read(&catalog_path).unwrap()).to_hex();
    assert_toml_keys(
        &fixture,
        &[
            "book_ids",
            "catalog_snapshot",
            "rank_selection",
            "schema",
            "selection_filter",
            "source_name",
            "sources",
        ],
    );
    assert_eq!(fixture["schema"].as_str(), Some("gutenberg_fixture.v1"));
    assert_eq!(fixture["source_name"].as_str(), Some("Project Gutenberg"));
    assert_toml_keys(
        &fixture["catalog_snapshot"],
        &[
            "last_modified_utc",
            "local_path",
            "observed_at_utc",
            "sha256",
            "size_bytes",
            "url",
        ],
    );
    assert_eq!(
        fixture["catalog_snapshot"]["observed_at_utc"].as_str(),
        Some("2026-05-19T00:00:00Z")
    );
    assert_eq!(
        fixture["catalog_snapshot"]["last_modified_utc"].as_str(),
        Some("2026-05-18T00:00:00Z")
    );
    assert_eq!(
        fixture["catalog_snapshot"]["sha256"].as_str(),
        Some(catalog_sha256.as_str())
    );
    assert_toml_keys(&fixture["selection_filter"], &["canonical_json", "sha256"]);
    let selection_filter_json = "{\"has_plain_text\":true,\"languages_canonical\":[\"en\"],\"pg_rights\":\"Public domain in the USA.\"}";
    let selection_filter_sha256 = sha256(selection_filter_json.as_bytes()).to_hex();
    assert_eq!(
        fixture["selection_filter"]["canonical_json"].as_str(),
        Some(selection_filter_json)
    );
    assert_eq!(
        fixture["selection_filter"]["sha256"].as_str(),
        Some(selection_filter_sha256.as_str())
    );
    assert_toml_keys(
        &fixture["rank_selection"],
        &[
            "book_count",
            "book_ids_self_hash_sha256",
            "candidates_total",
            "rank_prefix_ascii",
            "target_slice",
        ],
    );
    assert_eq!(
        fixture["rank_selection"]["rank_prefix_ascii"].as_str(),
        Some("gbf:s4:gutenberg-select:v1")
    );
    assert_eq!(
        fixture["rank_selection"]["book_count"].as_integer(),
        Some(4)
    );
    assert_toml_keys(&fixture["book_ids"], &["values"]);
    let sources = fixture["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 4);
    assert_toml_keys(
        &sources[0],
        &[
            "author",
            "book_id",
            "charset",
            "compression_kind",
            "extent_declared",
            "fetch_namespace_id",
            "fetch_namespace_kind",
            "local_blob_path",
            "media_type",
            "mirror_fetch_url",
            "pre_strip_utf8_sha256",
            "pre_strip_utf8_size_bytes",
            "preference_class",
            "rdf_resource_url",
            "selected_format",
            "source_blob_sha256",
            "source_blob_size_bytes",
            "source_landing_url",
            "title",
        ],
    );
    let selected_format = sources[0]["selected_format"].as_str().unwrap();
    let selected_format_lines = selected_format
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(
        selected_format_lines,
        vec![
            "text/plain".to_owned(),
            "utf-8".to_owned(),
            "none".to_owned(),
            String::new(),
            file_url(&plain_path),
        ]
    );
    assert_eq!(
        sources
            .iter()
            .map(|source| source["book_id"].as_integer().unwrap())
            .collect::<Vec<_>>(),
        vec![10, 20, 30, 40]
    );
    let latin1_utf8_sha256 = sha256("Café\n".as_bytes()).to_hex();
    assert_eq!(
        sources[1]["pre_strip_utf8_sha256"].as_str(),
        Some(latin1_utf8_sha256.as_str())
    );
    assert_eq!(sources[2]["compression_kind"].as_str(), Some("gzip"));
    assert_eq!(sources[3]["compression_kind"].as_str(), Some("zip"));
    assert_eq!(
        sources[3]["archive_member_path"].as_str(),
        Some("text/book40.txt")
    );
    for source in sources {
        let blob_path = Path::new(source["local_blob_path"].as_str().unwrap());
        let blob = std::fs::read(blob_path).expect("source blob reads");
        let blob_sha256 = sha256(&blob).to_hex();
        assert_eq!(
            source["source_blob_sha256"].as_str(),
            Some(blob_sha256.as_str())
        );
    }

    let first_fixture_hash = sha256(std::fs::read(&fixture_output).unwrap());
    let mut rerun = gbf();
    rerun.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&catalog_path),
        "--catalog-path",
        catalog_path.to_str().expect("utf8 catalog path"),
        "--catalog-observed-at-utc",
        "2026-05-19T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-18T00:00:00Z",
        "--cache-dir",
        corpus_dir.to_str().expect("utf8 corpus dir"),
        "--fixture-output",
        fixture_output.to_str().expect("utf8 fixture path"),
        "--target-slice",
        "4",
        "--fetch-namespace-kind",
        "local_private_mirror",
        "--fetch-namespace-id",
        "file://local-gutenberg-fixture/",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    rerun.assert().success();
    assert_eq!(
        sha256(std::fs::read(&fixture_output).unwrap()),
        first_fixture_hash
    );
}

#[test]
fn s4_harvest_rejects_existing_catalog_snapshot_pin_mismatch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let corpus_dir = temp.path().join("corpus");
    let catalog_path = temp.path().join("rdf-files.tar.bz2");
    write_catalog(
        &catalog_path,
        &[rdf_entry(
            10,
            "file:///tmp/unused-gutenberg-source.txt",
            "text/plain; charset=utf-8",
            12,
        )],
    );

    let fixture_output = temp.path().join("gutenberg.toml");
    std::fs::write(
        &fixture_output,
        r#"
schema = "gutenberg_fixture.v1"

[catalog_snapshot]
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
"#,
    )
    .expect("existing fixture writes");

    let output = temp.path().join("harvest.json");
    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&catalog_path),
        "--catalog-path",
        catalog_path.to_str().expect("utf8 catalog path"),
        "--catalog-observed-at-utc",
        "2026-05-19T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-18T00:00:00Z",
        "--cache-dir",
        corpus_dir.to_str().expect("utf8 corpus dir"),
        "--fixture-output",
        fixture_output.to_str().expect("utf8 fixture path"),
        "--target-slice",
        "1",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    command
        .assert()
        .failure()
        .stderr(predicate::str::contains("catalog snapshot SHA mismatch"));
    assert!(
        std::fs::read_to_string(&fixture_output)
            .expect("existing fixture still reads")
            .contains("0000000000000000000000000000000000000000000000000000000000000000"),
        "mismatched existing fixture pin must not be overwritten"
    );
}

#[test]
fn s4_harvest_captures_internal_structured_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let corpus_dir = temp.path().join("corpus");
    let retained_path = temp.path().join("plain-10.txt");
    let dropped_zip_path = temp.path().join("zip-20.zip");
    std::fs::write(&retained_path, b"Event UTF-8\n").expect("retained source writes");
    std::fs::write(&dropped_zip_path, zip_bytes("image.bin", b"PNG-ish\n"))
        .expect("dropped zip source writes");

    let catalog_path = temp.path().join("rdf-files.tar.bz2");
    write_catalog(
        &catalog_path,
        &[
            rdf_entry(
                10,
                &file_url(&retained_path),
                "text/plain; charset=utf-8",
                std::fs::metadata(&retained_path).unwrap().len(),
            ),
            rdf_entry(
                20,
                &file_url(&dropped_zip_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&dropped_zip_path).unwrap().len(),
            ),
        ],
    );

    let fixture_output = temp.path().join("gutenberg.toml");
    let output = temp.path().join("harvest.json");
    let events_path = temp.path().join("events.ndjson");
    let mut command = gbf();
    command.args([
        "--capture-events",
        events_path.to_str().expect("utf8 events path"),
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&catalog_path),
        "--catalog-path",
        catalog_path.to_str().expect("utf8 catalog path"),
        "--catalog-observed-at-utc",
        "2026-05-19T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-18T00:00:00Z",
        "--cache-dir",
        corpus_dir.to_str().expect("utf8 corpus dir"),
        "--fixture-output",
        fixture_output.to_str().expect("utf8 fixture path"),
        "--target-slice",
        "2",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    let output_result = command.output().expect("s4 harvest runs");
    assert!(
        output_result.status.success(),
        "harvest failed:\n{}",
        command_output(&output_result)
    );

    let records = read_ndjson_events(&events_path);
    let started = single_event_fields(&records, "s4_harvest_started");
    assert_eq!(
        started["catalog_url"].as_str(),
        Some(file_url(&catalog_path).as_str())
    );
    assert_eq!(started["target_slice"].as_u64(), Some(2));
    assert_eq!(started["network_permitted"].as_bool(), Some(true));

    let catalog_verified = single_event_fields(&records, "s4_harvest_catalog_snapshot_verified");
    assert_eq!(
        catalog_verified["fixture_pin_present"].as_bool(),
        Some(false)
    );
    assert!(
        catalog_verified["catalog_snapshot_sha256"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );

    let selected_books = event_fields(&records, "s4_harvest_book_selected");
    assert_eq!(
        selected_books
            .iter()
            .map(|fields| fields["book_id"].as_u64().expect("book id"))
            .collect::<Vec<_>>(),
        vec![10, 20]
    );

    let fetched_blobs = event_fields(&records, "s4_harvest_source_blob_fetched");
    assert_eq!(fetched_blobs.len(), 2);
    assert!(fetched_blobs.iter().all(|fields| {
        fields["source_blob_sha256"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
            && fields["source_blob_size_bytes"].as_u64().is_some()
            && fields["cache_hit"].as_bool() == Some(false)
    }));

    let drops = event_fields(&records, "s4_harvest_source_dropped");
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0]["book_id"].as_u64(), Some(20));
    assert_eq!(
        drops[0]["drop_reason"].as_str(),
        Some("no_plaintext_archive_member")
    );

    let finalized = single_event_fields(&records, "s4_harvest_finalized");
    assert_eq!(finalized["book_count"].as_u64(), Some(2));
    assert_eq!(finalized["source_count"].as_u64(), Some(2));
    assert_eq!(finalized["drop_count"].as_u64(), Some(1));
    assert!(
        finalized["fixture_sha256"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("sha256:"))
    );
}

#[test]
fn s4_harvest_records_per_book_drop_rows_and_strict_zip_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let corpus_dir = temp.path().join("corpus");
    let retained_zip_path = temp.path().join("zip-10.zip");
    let zip_no_plaintext_path = temp.path().join("zip-20.zip");
    let invalid_utf8_path = temp.path().join("invalid-30.txt");
    let invalid_raw_zip_path = temp.path().join("zip-50.zip");
    let ambiguous_zip_path = temp.path().join("zip-60.zip");
    let unsupported_charset_path = temp.path().join("unsupported-70.txt");
    let decomposed_member = "text/cafe\u{301}.txt";
    std::fs::write(
        &retained_zip_path,
        zip_bytes(decomposed_member, b"Zip UTF-8\n"),
    )
    .expect("retained zip source writes");
    std::fs::write(&zip_no_plaintext_path, zip_bytes("image.bin", b"PNG-ish\n"))
        .expect("non-plaintext zip source writes");
    std::fs::write(&invalid_utf8_path, b"\xff\n").expect("invalid utf8 source writes");
    std::fs::write(
        &invalid_raw_zip_path,
        zip_bytes_with_invalid_utf8_name(b"Invalid raw member path\n"),
    )
    .expect("invalid raw-name zip source writes");
    std::fs::write(
        &ambiguous_zip_path,
        zip_bytes_with_duplicate_members("text/tie.txt", b"left!\n", b"right\n"),
    )
    .expect("ambiguous zip source writes");
    std::fs::write(&unsupported_charset_path, b"Unsupported charset\n")
        .expect("unsupported charset source writes");

    let catalog_path = temp.path().join("rdf-files.tar.bz2");
    write_catalog(
        &catalog_path,
        &[
            rdf_entry(
                10,
                &file_url(&retained_zip_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&retained_zip_path).unwrap().len(),
            ),
            rdf_entry(
                20,
                &file_url(&zip_no_plaintext_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&zip_no_plaintext_path).unwrap().len(),
            ),
            rdf_entry(
                30,
                &file_url(&invalid_utf8_path),
                "text/plain; charset=utf-8",
                std::fs::metadata(&invalid_utf8_path).unwrap().len(),
            ),
            rdf_entry(
                40,
                "https://example.com/caf\u{e9}.txt",
                "text/plain; charset=utf-8",
                1,
            ),
            rdf_entry(
                50,
                &file_url(&invalid_raw_zip_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&invalid_raw_zip_path).unwrap().len(),
            ),
            rdf_entry(
                60,
                &file_url(&ambiguous_zip_path),
                "application/zip; charset=utf-8",
                std::fs::metadata(&ambiguous_zip_path).unwrap().len(),
            ),
            rdf_entry(
                70,
                &file_url(&unsupported_charset_path),
                "text/plain; charset=koi8-r",
                std::fs::metadata(&unsupported_charset_path).unwrap().len(),
            ),
        ],
    );

    let fixture_output = temp.path().join("gutenberg.toml");
    let output = temp.path().join("harvest.json");
    let mut command = gbf();
    command.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&catalog_path),
        "--catalog-path",
        catalog_path.to_str().expect("utf8 catalog path"),
        "--catalog-observed-at-utc",
        "2026-05-19T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-18T00:00:00Z",
        "--cache-dir",
        corpus_dir.to_str().expect("utf8 corpus dir"),
        "--fixture-output",
        fixture_output.to_str().expect("utf8 fixture path"),
        "--target-slice",
        "7",
        "--fetch-namespace-kind",
        "local_private_mirror",
        "--fetch-namespace-id",
        "file://local-gutenberg-fixture/",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    let output_result = command.output().expect("s4 harvest runs");
    assert!(
        output_result.status.success(),
        "harvest failed:\n{}",
        command_output(&output_result)
    );

    let evidence: Value =
        serde_json::from_slice(&std::fs::read(&output).expect("harvest evidence reads"))
            .expect("harvest evidence parses");
    assert_eq!(evidence["harvest"]["book_count"], 7);
    assert_eq!(evidence["harvest"]["source_count"], 5);

    let fixture_text = std::fs::read_to_string(&fixture_output).expect("fixture reads");
    let fixture: toml::Value = toml::from_str(&fixture_text).expect("fixture parses");
    let sources = fixture["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 7);
    let by_id = sources_by_book_id(sources);

    let retained = by_id.get(&10).expect("book 10 source row");
    assert_eq!(retained.get("drop_reason"), None);
    assert_eq!(retained["compression_kind"].as_str(), Some("zip"));
    assert_eq!(
        retained["archive_member_path"].as_str(),
        Some("text/caf\u{e9}.txt")
    );

    let no_plaintext = by_id.get(&20).expect("book 20 source row");
    assert_eq!(
        no_plaintext["drop_reason"].as_str(),
        Some("no_plaintext_archive_member")
    );
    assert_eq!(no_plaintext["compression_kind"].as_str(), Some("zip"));
    assert!(no_plaintext.get("source_blob_sha256").is_some());
    assert_eq!(no_plaintext.get("archive_member_path"), None);
    assert_eq!(no_plaintext.get("pre_strip_utf8_sha256"), None);

    let invalid_utf8 = by_id.get(&30).expect("book 30 source row");
    assert_eq!(invalid_utf8["drop_reason"].as_str(), Some("invalid_utf8"));
    assert!(invalid_utf8.get("source_blob_sha256").is_some());
    assert!(invalid_utf8.get("selected_format").is_some());
    assert_eq!(invalid_utf8.get("pre_strip_utf8_sha256"), None);

    let lossy_url = by_id.get(&40).expect("book 40 source row");
    assert_eq!(
        lossy_url["drop_reason"].as_str(),
        Some("no_supported_plaintext_format")
    );
    assert_eq!(lossy_url.get("mirror_fetch_url"), None);
    assert_eq!(lossy_url.get("source_blob_sha256"), None);

    let invalid_raw_name = by_id.get(&50).expect("book 50 source row");
    assert_eq!(
        invalid_raw_name["drop_reason"].as_str(),
        Some("no_plaintext_archive_member")
    );
    assert_eq!(invalid_raw_name["compression_kind"].as_str(), Some("zip"));
    assert_eq!(invalid_raw_name.get("archive_member_path"), None);

    let ambiguous = by_id.get(&60).expect("book 60 source row");
    assert_eq!(
        ambiguous["drop_reason"].as_str(),
        Some("ambiguous_plaintext_archive")
    );
    assert_eq!(ambiguous["compression_kind"].as_str(), Some("zip"));
    assert!(ambiguous.get("source_blob_sha256").is_some());
    assert_eq!(ambiguous.get("pre_strip_utf8_sha256"), None);

    let unsupported_charset = by_id.get(&70).expect("book 70 source row");
    assert_eq!(
        unsupported_charset["drop_reason"].as_str(),
        Some("no_supported_plaintext_format")
    );
    assert_eq!(unsupported_charset.get("source_blob_sha256"), None);
    assert_eq!(unsupported_charset.get("selected_format"), None);
}

#[test]
fn s4_harvest_rejects_bad_catalog_timestamps_before_fetch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let output = temp.path().join("harvest.json");
    let missing_catalog = temp.path().join("missing-rdf-files.tar.bz2");

    let mut invalid = gbf();
    invalid.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&missing_catalog),
        "--catalog-observed-at-utc",
        "not-rfc3339",
        "--target-slice",
        "1",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    invalid.assert().failure().stderr(predicate::str::contains(
        "catalog_observed_at_utc must be RFC3339",
    ));

    let mut reversed = gbf();
    reversed.args([
        "--log-level",
        "off",
        "s4",
        "harvest-gutenberg-fixture",
        "--network-permitted",
        "--catalog-url",
        &file_url(&missing_catalog),
        "--catalog-observed-at-utc",
        "2026-05-18T00:00:00Z",
        "--catalog-last-modified-utc",
        "2026-05-19T00:00:00Z",
        "--target-slice",
        "1",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    reversed
        .assert()
        .failure()
        .stderr(predicate::str::contains("must be >="));
}

#[test]
fn s4_existing_gutenberg_fixture_rank_hash_matches_python_prototype_formula() {
    let root = workspace_root();
    let fixture_text = std::fs::read_to_string(root.join("fixtures/corpora/gutenberg.toml"))
        .expect("gutenberg fixture reads");
    let fixture: toml::Value = toml::from_str(&fixture_text).expect("gutenberg fixture parses");
    assert_eq!(fixture["schema"].as_str(), Some("gutenberg_fixture.v1"));
    assert_eq!(
        fixture["rank_selection"]["rank_prefix_ascii"].as_str(),
        Some("gbf:s4:gutenberg-select:v1")
    );
    assert_eq!(
        fixture["rank_selection"]["target_slice"].as_integer(),
        Some(1500)
    );

    let book_ids = fixture["book_ids"]["values"]
        .as_array()
        .expect("book ids array")
        .iter()
        .map(|value| value.as_integer().expect("integer book id"))
        .collect::<Vec<_>>();
    assert_eq!(book_ids.len(), 1500);
    assert!(
        book_ids.windows(2).all(|pair| pair[0] < pair[1]),
        "book_ids must be sorted ascending and deduplicated"
    );
    let joined = book_ids
        .iter()
        .map(i64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let observed_rank_hash = sha256(joined.as_bytes()).to_hex();
    assert_eq!(
        fixture["rank_selection"]["book_ids_self_hash_sha256"].as_str(),
        Some(observed_rank_hash.as_str())
    );
    assert_eq!(
        observed_rank_hash,
        "98ed3f1b494f8f439d5f879bde8af7f57d94bf655132794339cf7f497967d289"
    );

    let prototype =
        std::fs::read_to_string(root.join("scripts/corpus/gutenberg/select_book_ids.py"))
            .expect("python prototype reads");
    assert!(prototype.contains("RANK_PREFIX = b\"gbf:s4:gutenberg-select:v1\""));
    assert!(prototype.contains("struct.pack(\"<I\", book_id)"));
    assert!(prototype.contains("\",\".join(str(i) for i in book_ids)"));
}

#[test]
fn s4_cli_captures_lifecycle_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = temp.path().join("events.ndjson");
    let output = temp.path().join("replay.json");

    let mut command = gbf();
    command.args([
        "--capture-events",
        events.to_str().expect("utf8 events path"),
        "s4",
        "replay-full",
        "--seed-list",
        "0",
        "--output",
        output.to_str().expect("utf8 output path"),
    ]);
    command.assert().success();

    let events = std::fs::read_to_string(events).expect("events read");
    assert!(events.contains("\"event_name\":\"s4_cli_verb_started\""));
    assert!(events.contains("\"event_name\":\"s4_cli_verb_finalized\""));
    assert!(events.contains("\"verb\":\"replay-full\""));
}

#[test]
fn s4_cli_feature_forwarding_is_registered() {
    let manifest = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("gbf-cli Cargo.toml reads");

    assert!(
        manifest.contains("s4 = [\"gbf-experiments/s4\"]"),
        "gbf-cli must forward the base s4 feature"
    );
    assert!(
        manifest.contains("s4-full = [\"s4\", \"gbf-experiments/s4-full\"]"),
        "gbf-cli must forward s4-full through gbf-experiments"
    );
    assert!(
        manifest.contains("s4-falsify = [\"s4\", \"gbf-experiments/s4-falsify\"]"),
        "gbf-cli must forward s4-falsify through gbf-experiments"
    );
}

fn single_stdout_hash(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        1,
        "stdout must be one pipeable line:\n{stdout}"
    );
    let line = lines[0];
    assert!(
        line.strip_prefix("sha256:")
            .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit())),
        "stdout must be a sha256 self-hash line, got {line:?}"
    );
    line.to_owned()
}

fn command_output(output: &Output) -> String {
    format!(
        "status: {}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn read_ndjson_events(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .expect("events read")
        .lines()
        .map(|line| serde_json::from_str(line).expect("event JSON parses"))
        .collect()
}

fn event_fields<'a>(records: &'a [Value], event_name: &str) -> Vec<&'a Value> {
    records
        .iter()
        .filter_map(|record| {
            let fields = record.get("fields")?;
            (fields.get("event_name").and_then(Value::as_str) == Some(event_name)).then_some(fields)
        })
        .collect()
}

fn single_event_fields<'a>(records: &'a [Value], event_name: &str) -> &'a Value {
    let events = event_fields(records, event_name);
    assert_eq!(
        events.len(),
        1,
        "expected exactly one {event_name} event, got {events:?}"
    );
    events[0]
}

fn assert_toml_keys(value: &toml::Value, expected: &[&str]) {
    let observed = value
        .as_table()
        .expect("TOML value is a table")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(observed, expected);
}

fn sources_by_book_id(sources: &[toml::Value]) -> BTreeMap<i64, &toml::Value> {
    sources
        .iter()
        .map(|source| {
            (
                source["book_id"].as_integer().expect("source book_id"),
                source,
            )
        })
        .collect()
}

fn file_url(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn promotion_hash(salt: u8) -> Hash256 {
    Hash256::from_bytes([salt; 32])
}

fn promotion_manifest(unmappable_rate_corpus: f64) -> GutenbergManifest {
    let mut manifest = GutenbergManifest {
        schema: GutenbergManifest::schema_id(),
        source_name: GutenbergManifest::source_name_literal(),
        catalog_snapshot_url: "file://fixtures/gutenberg-rdf.tar.bz2".to_owned(),
        catalog_snapshot_sha256: promotion_hash(1),
        catalog_snapshot_observed_at_utc: "2026-05-19T00:00:00Z".to_owned(),
        catalog_snapshot_last_modified_utc: None,
        selection_filter_canonical_json: "{}".to_owned(),
        selection_filter_sha256: promotion_hash(2),
        book_ids: vec![1001, 1002],
        sources: vec![
            promotion_source_record(1001, GutenbergSplit::Train, 20),
            promotion_source_record(1002, GutenbergSplit::Val, 30),
        ],
        header_regex_pattern: "START".to_owned(),
        footer_regex_pattern: "END".to_owned(),
        normalization_spec_self_hash: promotion_hash(3),
        dedup_policy: GutenbergDedupPolicy::exact_post_strip_charset_body_sha(),
        split_seed_u128: "00000000000000000000000000000001".to_owned(),
        split_train_fraction: 0.90,
        split_val_fraction: 0.10,
        train_path: "experiments/S4/corpus/gutenberg_train.bin".to_owned(),
        val_path: "experiments/S4/corpus/gutenberg_val.bin".to_owned(),
        train_sha256: promotion_hash(4),
        val_sha256: promotion_hash(5),
        train_byte_length: 128,
        val_byte_length: 128,
        train_book_count: 1,
        val_book_count: 1,
        drop_count_total: 0,
        drop_count_no_supported_plaintext_format: 0,
        drop_count_no_plaintext_archive_member: 0,
        drop_count_source_decode_failed: 0,
        drop_count_ambiguous_plaintext_archive: 0,
        drop_count_invalid_utf8: 0,
        drop_count_empty_after_strip: 0,
        drop_count_marker_missing: 0,
        drop_count_unmappable_density: 0,
        drop_count_dedup_collision: 0,
        unmappable_rate_corpus,
        raw_byte_policy: GutenbergManifest::raw_byte_policy_literal(),
        retained_book_count_min: 2,
        manifest_self_hash: Hash256::ZERO,
    };
    manifest.manifest_self_hash = manifest
        .compute_self_hash()
        .expect("promotion manifest self-hash");
    manifest
}

fn promotion_source_record(book_id: u32, split: GutenbergSplit, salt: u8) -> GutenbergSourceRecord {
    GutenbergSourceRecord {
        book_id,
        title: format!("Book {book_id}"),
        author: "Fixture Author".to_owned(),
        source_landing_url: format!("https://www.gutenberg.org/ebooks/{book_id}"),
        mirror_fetch_url: None,
        mirror_snapshot_id: None,
        selected_format: Some("text/plain\nutf-8\nnone\n\nfile://fixture".to_owned()),
        source_blob_sha256: Some(promotion_hash(salt)),
        pre_strip_utf8_sha256: Some(promotion_hash(salt + 1)),
        license: GutenbergSourceRecord::public_domain_in_usa_license(),
        fetch_namespace_kind: Some(GutenbergFetchNamespaceKind::ContentAddressedCache),
        fetch_namespace_id: Some("fixture-cache".to_owned()),
        compression_kind: Some(GutenbergCompressionKind::None),
        archive_member_path: None,
        pre_strip_byte_length: Some(160),
        drop_reason: None,
        duplicate_of_book_id: None,
        post_strip_byte_length: Some(128),
        post_strip_sha256: Some(promotion_hash(salt + 2)),
        post_charset_body_sha256: Some(promotion_hash(salt + 3)),
        post_charset_token_length: Some(128),
        unmappable_count: Some(0),
        unmappable_density: Some(0.0),
        split: Some(split),
    }
}

fn write_json_artifact<T>(path: &Path, artifact: &T)
where
    T: Serialize,
{
    let bytes = CanonicalJson::to_vec(artifact).expect("promotion fixture canonicalizes");
    std::fs::write(path, bytes).expect("promotion fixture writes");
}

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-cli has a workspace parent")
        .to_path_buf()
}

fn gzip_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).expect("gzip write");
    encoder.finish().expect("gzip finish")
}

fn zip_bytes(path: &str, bytes: &[u8]) -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    zip.start_file(path, FileOptions::default())
        .expect("zip start file");
    zip.write_all(bytes).expect("zip write");
    zip.finish().expect("zip finish").into_inner()
}

fn zip_bytes_with_duplicate_members(path: &str, left: &[u8], right: &[u8]) -> Vec<u8> {
    assert_eq!(
        left.len(),
        right.len(),
        "ambiguous archive fixture members must tie on size"
    );
    let cursor = Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(cursor);
    zip.start_file(path, FileOptions::default())
        .expect("zip left start file");
    zip.write_all(left).expect("zip left write");
    zip.start_file(path, FileOptions::default())
        .expect("zip right start file");
    zip.write_all(right).expect("zip right write");
    zip.finish().expect("zip finish").into_inner()
}

fn zip_bytes_with_invalid_utf8_name(bytes: &[u8]) -> Vec<u8> {
    let mut zip = zip_bytes("bad.txt", bytes);
    let mut index = 0;
    while let Some(offset) = zip[index..]
        .windows("bad.txt".len())
        .position(|window| window == b"bad.txt")
    {
        index += offset;
        zip[index] = 0xff;
        index += "bad.txt".len();
    }
    zip
}

fn write_catalog(path: &Path, entries: &[(u32, String)]) {
    let file = std::fs::File::create(path).expect("catalog creates");
    let encoder = BzEncoder::new(file, bzip2::Compression::best());
    let mut tar = tar::Builder::new(encoder);
    for (book_id, rdf) in entries {
        let mut header = Header::new_gnu();
        header.set_size(rdf.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("cache/epub/{book_id}/pg{book_id}.rdf"),
            rdf.as_bytes(),
        )
        .expect("rdf appends");
    }
    tar.finish().expect("tar finishes");
}

fn rdf_entry(book_id: u32, url: &str, media_type: &str, extent: u64) -> (u32, String) {
    (
        book_id,
        format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:pgterms="http://www.gutenberg.org/2009/pgterms/">
  <pgterms:ebook rdf:about="ebooks/{book_id}">
    <dcterms:title>Fixture Book {book_id}</dcterms:title>
    <dcterms:creator><pgterms:agent><pgterms:name>Fixture Author {book_id}</pgterms:name></pgterms:agent></dcterms:creator>
    <dcterms:language><rdf:Description><rdf:value>en</rdf:value></rdf:Description></dcterms:language>
    <dcterms:rights>Public domain in the USA.</dcterms:rights>
    <dcterms:hasFormat>
      <pgterms:file rdf:about="{url}">
        <dcterms:extent>{extent}</dcterms:extent>
        <dcterms:format><rdf:Description><rdf:value>{media_type}</rdf:value></rdf:Description></dcterms:format>
      </pgterms:file>
    </dcterms:hasFormat>
  </pgterms:ebook>
</rdf:RDF>"#
        ),
    )
}
