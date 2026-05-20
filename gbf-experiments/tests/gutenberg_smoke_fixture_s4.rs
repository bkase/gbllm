#![cfg(feature = "s4")]

mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use gbf_artifact::{BOS_ID, EOS_ID, GutenbergDropReason, GutenbergManifest};
use gbf_data::{GutenbergD3DropReason, strip_gutenberg_d3};
use gbf_experiments::s4::corpus_quality::S4CorpusQuality;
use gbf_experiments::s4::corpus_quality::{
    GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX, S4_UNMAPPABLE_GATE_DOC_EVENT,
    S4_UNMAPPABLE_GATE_OUTCOME_EVENT, S4UnmappableGateError, verify_gutenberg_unmappable_gate,
};
use gbf_experiments::s4::manifest::{
    GutenbergBuildError, GutenbergBuildOptions, GutenbergBuildSummary, build_gutenberg_corpus,
};
use gbf_foundation::sha256 as gbf_sha256;
use serde_json::json;
use sha2::{Digest, Sha256};
use toml::Value;

const SMOKE_MANIFEST: &str = "fixtures/corpora/gutenberg_smoke.toml";
const SMOKE_DIR: &str = "fixtures/corpora/gutenberg_smoke";
const SMOKE_EXPECTED: &str = "fixtures/corpora/gutenberg_smoke/expected.toml";
const MAX_SMOKE_BYTES: u64 = 5 * 1024 * 1024;

#[test]
fn gutenberg_smoke_fixture_is_hash_pinned_and_offline() {
    let root = workspace_root();
    let manifest = read_manifest(&root);
    let sources = sources(&manifest);

    assert_eq!(
        string_field(&manifest, "schema"),
        "gutenberg_smoke_fixture.v1"
    );
    assert_eq!(string_field(&manifest, "source_name"), "Project Gutenberg");
    assert_eq!(
        integer_field(&manifest, "book_count") as usize,
        sources.len()
    );
    assert!(
        (8..=10).contains(&sources.len()),
        "F-S4.18 smoke fixture should stay tiny but representative"
    );

    let mut observed_book_ids = Vec::with_capacity(sources.len());
    let mut observed_paths = BTreeSet::new();
    let mut observed_total_bytes = 0_u64;
    let mut observed_header_variants = BTreeMap::<String, u64>::new();

    for source in sources {
        let book_id = integer_field(source, "book_id") as u32;
        observed_book_ids.push(book_id);

        assert_eq!(
            string_field(source, "source_landing_url"),
            format!("https://www.gutenberg.org/ebooks/{book_id}")
        );
        assert!(
            !source
                .as_table()
                .expect("source is a table")
                .contains_key("mirror_fetch_url"),
            "smoke fixture must not require a network mirror fetch URL"
        );
        assert_eq!(
            string_field(source, "media_type"),
            "text/plain; charset=utf-8"
        );
        assert_eq!(string_field(source, "charset"), "utf-8");
        assert_eq!(integer_field(source, "preference_class"), 1);

        let local_blob_path = string_field(source, "local_blob_path");
        assert!(
            local_blob_path.starts_with(SMOKE_DIR),
            "local blob path must stay under {SMOKE_DIR}: {local_blob_path}"
        );
        assert!(
            !local_blob_path.contains("..") && !Path::new(&local_blob_path).is_absolute(),
            "local blob path must be a repository-relative fixture path"
        );
        assert!(
            observed_paths.insert(local_blob_path.clone()),
            "duplicate local blob path {local_blob_path}"
        );

        let sha256_hex = string_field(source, "source_blob_sha256");
        assert_lower_sha256_hex(&sha256_hex);
        let blob_path = root.join(&local_blob_path);
        let blob = std::fs::read(&blob_path).unwrap_or_else(|error| {
            panic!(
                "fixture blob {} reads: {error}",
                blob_path
                    .strip_prefix(&root)
                    .unwrap_or(&blob_path)
                    .display()
            )
        });
        observed_total_bytes += blob.len() as u64;

        assert_eq!(
            blob.len() as i64,
            integer_field(source, "source_blob_size_bytes")
        );
        assert_eq!(sha256_hex_bytes(&blob), sha256_hex);
        assert!(
            local_blob_path.contains(&format!("{book_id}.")),
            "blob path should name the book id for reviewability"
        );
        assert!(
            local_blob_path.contains(&sha256_hex[..16]),
            "blob path should include a short content-hash prefix"
        );

        let header_variant = string_field(source, "header_variant");
        assert!(
            matches!(header_variant.as_str(), "THIS" | "THE" | "BARE"),
            "unexpected header variant {header_variant:?}"
        );
        *observed_header_variants.entry(header_variant).or_default() += 1;
    }

    assert_eq!(
        observed_book_ids,
        vec![4089, 4493, 17125, 17422, 17424, 19394, 29156, 78663]
    );
    assert!(observed_book_ids.windows(2).all(|pair| pair[0] < pair[1]));
    assert_eq!(
        observed_total_bytes as i64,
        integer_field(&manifest, "total_bytes")
    );
    assert!(
        observed_total_bytes <= MAX_SMOKE_BYTES,
        "smoke fixture must stay below the offline CI size budget"
    );

    let coverage = manifest
        .get("header_variant_coverage")
        .expect("header_variant_coverage table");
    for variant in ["THIS", "THE", "BARE"] {
        assert_eq!(
            observed_header_variants.get(variant).copied().unwrap_or(0) as i64,
            integer_field(coverage, variant)
        );
    }
}

#[test]
fn gutenberg_smoke_expected_oracle_matches_fixture() {
    verify_smoke_expected(&workspace_root()).expect("expected.toml drift oracle matches fixture");
}

#[test]
fn gutenberg_smoke_expected_oracle_rejects_missing_expected_field() {
    let root = workspace_root();
    let temp = copy_smoke_fixture_to_temp(&root);
    let expected_path = temp.path().join(SMOKE_EXPECTED);
    let expected = std::fs::read_to_string(&expected_path).expect("temp expected reads");
    let mutated = expected.replace("drop_count_marker_missing = 0\n", "");
    assert_ne!(mutated, expected, "expected mutation should remove a field");
    std::fs::write(&expected_path, mutated).expect("temp expected mutates");

    let error = verify_smoke_expected(temp.path()).expect_err("missing field should fail");
    assert!(
        error.contains("drop_count_marker_missing"),
        "unexpected error: {error}"
    );
}

#[test]
fn gutenberg_smoke_expected_oracle_rejects_mutated_blob() {
    let root = workspace_root();
    let temp = copy_smoke_fixture_to_temp(&root);
    let blob_path = temp
        .path()
        .join("fixtures/corpora/gutenberg_smoke/4089.318e6b2f8aa79d2e.bin");
    let mut blob = std::fs::read(&blob_path).expect("temp blob reads");
    blob[0] ^= 0x01;
    std::fs::write(&blob_path, blob).expect("temp blob mutates");

    let error = verify_smoke_expected(temp.path()).expect_err("mutated blob should fail");
    assert!(
        error.contains("source blob sha256 drift for book 4089"),
        "unexpected error: {error}"
    );
}

#[test]
fn gutenberg_smoke_fixture_blobs_are_gutenberg_shaped_plaintext() {
    let root = workspace_root();
    let manifest = read_manifest(&root);

    for source in sources(&manifest) {
        let book_id = integer_field(source, "book_id");
        let blob_path = root.join(string_field(source, "local_blob_path"));
        let blob = std::fs::read(&blob_path).expect("fixture blob reads");
        let text = std::str::from_utf8(strip_utf8_bom(&blob)).unwrap_or_else(|error| {
            panic!("fixture book {book_id} must be UTF-8 plaintext: {error}")
        });
        assert!(
            text.contains("*** START OF"),
            "fixture book {book_id} should contain a Project Gutenberg start marker"
        );
        assert!(
            text.contains("*** END OF"),
            "fixture book {book_id} should contain a Project Gutenberg end marker"
        );
        assert!(
            text.lines().count() > 20,
            "fixture book {book_id} should be large enough to exercise corpus readers"
        );
    }
}

#[test]
fn gutenberg_build_corpus_smoke_emits_manifest_quality_and_split_streams() {
    let root = workspace_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("gutenberg-manifest.json");
    let train_path = temp.path().join("gutenberg-train.bin");
    let val_path = temp.path().join("gutenberg-val.bin");
    let corpus_quality_path = temp.path().join("corpus-quality.json");
    let summary = build_gutenberg_corpus(&GutenbergBuildOptions {
        fixture_path: root.join(SMOKE_MANIFEST),
        manifest_path: manifest_path.clone(),
        train_path: train_path.clone(),
        val_path: val_path.clone(),
        corpus_quality_path: Some(corpus_quality_path.clone()),
        tinystories_manifest_path: Some(root.join("fixtures/corpora/tinystories.toml")),
    })
    .expect("smoke fixture builds offline corpus artifacts");

    let manifest_bytes = std::fs::read(&manifest_path).expect("manifest reads");
    let manifest: GutenbergManifest =
        serde_json::from_slice(&manifest_bytes).expect("manifest parses");
    manifest
        .validate_canonical_write()
        .expect("manifest self-hash validates");
    assert_eq!(summary.manifest_self_hash, manifest.manifest_self_hash);
    assert_eq!(manifest.train_book_count, 7);
    assert_eq!(manifest.val_book_count, 1);
    assert_eq!(manifest.drop_count_total, 0);
    assert_eq!(manifest.drop_count_dedup_collision, 0);

    let train_bytes = std::fs::read(&train_path).expect("train stream reads");
    let val_bytes = std::fs::read(&val_path).expect("val stream reads");
    assert_eq!(gbf_sha256(&train_bytes), manifest.train_sha256);
    assert_eq!(gbf_sha256(&val_bytes), manifest.val_sha256);
    assert_eq!(
        train_bytes.iter().filter(|id| **id == BOS_ID).count(),
        manifest.train_book_count as usize
    );
    assert_eq!(
        train_bytes.iter().filter(|id| **id == EOS_ID).count(),
        manifest.train_book_count as usize
    );
    assert_eq!(
        val_bytes.iter().filter(|id| **id == BOS_ID).count(),
        manifest.val_book_count as usize
    );
    assert_eq!(
        val_bytes.iter().filter(|id| **id == EOS_ID).count(),
        manifest.val_book_count as usize
    );

    let quality_bytes = std::fs::read(&corpus_quality_path).expect("quality reads");
    let quality: S4CorpusQuality = serde_json::from_slice(&quality_bytes).expect("quality parses");
    quality
        .validate_canonical_write()
        .expect("quality self-hash validates");
    assert_eq!(
        quality.gutenberg_manifest_self_hash,
        manifest.manifest_self_hash
    );
    assert_eq!(quality.drop_counts.total, 0);
    assert_eq!(quality.kn_baseline_pointer.owner_bead, "bd-2nca");
    assert_eq!(quality.contamination_outcome_pointer.owner_bead, "bd-2p3n");
    assert_eq!(quality.per_corpus.len(), 1);
    assert_eq!(quality.per_corpus[0].corpus_id, "Gutenberg");
    assert!(quality.per_corpus[0].tokens_per_doc_max > 0);
    assert_eq!(quality.retained_book_count, 8);
    assert_eq!(quality.unmappable_gate.status, "passed");
    assert!(quality.unmappable_gate.aggregate_passed);
    assert_eq!(
        quality.unmappable_gate.retained_book_count,
        quality.retained_book_count
    );
    assert_eq!(
        quality.unmappable_gate.max_unmappable_rate_corpus,
        GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX
    );
    assert_eq!(quality.unmappable_gate.retained_document_densities.len(), 8);
    for doc in &quality.unmappable_gate.retained_document_densities {
        assert!(doc.passed);
        assert!(doc.unmappable_density <= doc.max_unmappable_density_per_doc);
        assert!(doc.body_token_count > 0);
    }
    let quality_json: serde_json::Value =
        serde_json::from_slice(&quality_bytes).expect("quality JSON parses");
    assert_eq!(quality_json["unmappable_gate"]["status"], json!("passed"));
    assert_eq!(
        quality_json["unmappable_gate"]["aggregate_passed"],
        json!(true)
    );
}

#[test]
fn gutenberg_build_corpus_is_deterministic_across_two_runs() {
    let root = workspace_root();
    let temp = tempfile::tempdir().expect("tempdir");
    let options = GutenbergBuildOptions {
        fixture_path: root.join(SMOKE_MANIFEST),
        manifest_path: temp.path().join("gutenberg-manifest.json"),
        train_path: temp.path().join("gutenberg-train.bin"),
        val_path: temp.path().join("gutenberg-val.bin"),
        corpus_quality_path: Some(temp.path().join("corpus-quality.json")),
        tinystories_manifest_path: Some(root.join("fixtures/corpora/tinystories.toml")),
    };
    let first = build_with_options(options.clone(), temp.path()).expect("first build succeeds");
    let second = build_with_options(options, temp.path()).expect("second build succeeds");

    assert_eq!(
        first.summary.manifest_self_hash,
        second.summary.manifest_self_hash
    );
    assert_eq!(
        first.summary.corpus_quality_self_hash,
        second.summary.corpus_quality_self_hash
    );
    assert_eq!(first.summary.train_sha256, second.summary.train_sha256);
    assert_eq!(first.summary.val_sha256, second.summary.val_sha256);
    assert_eq!(first.manifest_bytes, second.manifest_bytes);
    assert_eq!(first.train_bytes, second.train_bytes);
    assert_eq!(first.val_bytes, second.val_bytes);
    assert_eq!(first.quality_bytes, second.quality_bytes);
}

#[test]
fn gutenberg_build_corpus_rejects_source_blob_sha_mismatch_with_book_id() {
    let root = workspace_root();
    let temp = copy_smoke_fixture_to_temp(&root);
    let blob_path = temp
        .path()
        .join("fixtures/corpora/gutenberg_smoke/4089.318e6b2f8aa79d2e.bin");
    let mut blob = std::fs::read(&blob_path).expect("temp blob reads");
    blob[0] ^= 0x01;
    std::fs::write(&blob_path, blob).expect("temp blob mutates");

    let error = build_with_fixture(temp.path().join(SMOKE_MANIFEST), temp.path())
        .expect_err("source blob mismatch should abort build-corpus");
    assert!(
        matches!(
            error,
            GutenbergBuildError::SourceBlobShaMismatch { book_id: 4089, .. }
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn gutenberg_build_corpus_enforces_retained_floor() {
    let root = workspace_root();
    let temp = copy_smoke_fixture_to_temp(&root);
    let fixture_path = temp.path().join(SMOKE_MANIFEST);
    let mut fixture = std::fs::read_to_string(&fixture_path).expect("temp fixture reads");
    fixture.push_str("\n[guards]\nretained_book_count_min = 9\n");
    std::fs::write(&fixture_path, fixture).expect("temp fixture writes");

    let error = build_with_fixture(fixture_path, temp.path())
        .expect_err("retained floor breach should abort build-corpus");
    assert!(
        matches!(
            error,
            GutenbergBuildError::RetainedBookCountBelowFloor {
                retained_book_count: 8,
                retained_book_count_min: 9
            }
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn gutenberg_build_corpus_drops_high_per_doc_unmappable_density() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = write_synthetic_fixture(
        temp.path(),
        "per_doc_unmappable",
        2,
        &[
            SyntheticBook::new(4089, format!("{}{}", "a".repeat(979), "@".repeat(21))),
            SyntheticBook::new(4493, format!("{}b", "a".repeat(999))),
            SyntheticBook::new(78663, format!("{}c", "a".repeat(999))),
        ],
    );

    let result = build_with_fixture(fixture, temp.path())
        .expect("per-document unmappable breach is a recoverable drop");
    assert_eq!(result.summary.drop_count_total, 1);
    let manifest: GutenbergManifest =
        serde_json::from_slice(&result.manifest_bytes).expect("manifest parses");
    assert_eq!(manifest.drop_count_unmappable_density, 1);
    let dropped = manifest
        .sources
        .iter()
        .find(|source| source.book_id == 4089)
        .expect("synthetic dropped source exists");
    assert_eq!(
        dropped.drop_reason,
        Some(GutenbergDropReason::UnmappableDensityHigh)
    );
    assert!(
        dropped.unmappable_density.expect("density recorded") > 0.02,
        "dropped source should preserve the measured over-threshold density"
    );
}

#[test]
fn gutenberg_build_corpus_hard_fails_aggregate_unmappable_rate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = write_synthetic_fixture(
        temp.path(),
        "aggregate_unmappable",
        2,
        &[
            SyntheticBook::new(4089, format!("{}{}", "a".repeat(994), "@".repeat(6))),
            SyntheticBook::new(78663, format!("{}b{}", "a".repeat(993), "@".repeat(6))),
        ],
    );

    let error = build_with_fixture(fixture, temp.path())
        .expect_err("aggregate unmappable breach should abort build-corpus");
    assert!(
        matches!(
            error,
            GutenbergBuildError::CorpusGate(S4UnmappableGateError::CorpusUnmappableRateHigh {
                max_rate: GUTENBERG_UNMAPPABLE_CORPUS_RATE_MAX,
                unmappable_count: 12,
                body_token_count: 2000,
                ..
            })
        ),
        "unexpected error: {error}"
    );
}

#[test]
fn gutenberg_unmappable_verifier_excludes_bos_eos_from_density() {
    let temp = tempfile::tempdir().expect("tempdir");
    let fixture = write_synthetic_fixture(
        temp.path(),
        "bos_eos_exclusion",
        2,
        &[
            SyntheticBook::new(4089, format!("{}{}", "a".repeat(994), "@".repeat(6))),
            SyntheticBook::new(78663, format!("{}b", "a".repeat(999))),
        ],
    );
    let result = build_with_fixture(fixture, temp.path()).expect("fixture builds");
    let manifest: GutenbergManifest =
        serde_json::from_slice(&result.manifest_bytes).expect("manifest parses");
    let report = verify_gutenberg_unmappable_gate(&manifest).expect("D5 gate passes");

    assert_eq!(report.unmappable_count, 6);
    assert_eq!(report.body_token_count, 2000);
    assert_eq!(report.unmappable_rate_corpus, 0.003);
    assert_eq!(
        result
            .train_bytes
            .iter()
            .filter(|id| **id == BOS_ID)
            .count(),
        1
    );
    assert_eq!(
        result.val_bytes.iter().filter(|id| **id == EOS_ID).count(),
        1
    );
}

#[test]
fn gutenberg_build_corpus_emits_structured_pipeline_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut books = vec![
        SyntheticBook::marker_missing(1),
        SyntheticBook::new(2, "a".repeat(1000)),
        SyntheticBook::new(3, "a".repeat(1000)),
        SyntheticBook::new(4, format!("{}{}", "a".repeat(979), "@".repeat(21))),
    ];
    for book_id in 5..=20 {
        books.push(SyntheticBook::new(
            book_id,
            format!("{}{}", "a".repeat(996), format!("{book_id:04}")),
        ));
    }
    let fixture = write_synthetic_fixture(temp.path(), "events", 2, &books);
    let capture = common::tracing_capture::TraceCapture::default();
    let result = common::tracing_capture::with_trace_capture(&capture, || {
        build_with_fixture(fixture, temp.path()).expect("event fixture builds")
    });
    assert_eq!(result.summary.drop_count_total, 3);

    let events = common::tracing_capture::captured_events(&capture);
    for event_name in [
        "s4_build_corpus_started",
        "s4_blob_sha256_verified",
        "s4_strip_drop",
        "s4_charset_drop",
        S4_UNMAPPABLE_GATE_DOC_EVENT,
        S4_UNMAPPABLE_GATE_OUTCOME_EVENT,
        "s4_split_assigned",
        "s4_dedup_drop",
        "s4_manifest_finalized",
    ] {
        assert!(
            events.iter().any(|event| event.name == event_name),
            "missing event {event_name}; saw {events:#?}"
        );
    }
    let gate_outcome = events
        .iter()
        .find(|event| event.name == S4_UNMAPPABLE_GATE_OUTCOME_EVENT)
        .expect("unmappable gate outcome event");
    assert_eq!(gate_outcome.fields.get("status"), Some(&json!("passed")));
    assert_eq!(
        gate_outcome.fields.get("aggregate_passed"),
        Some(&json!(true))
    );
    let doc_event = events
        .iter()
        .find(|event| event.name == S4_UNMAPPABLE_GATE_DOC_EVENT)
        .expect("unmappable gate doc event");
    assert!(doc_event.fields.contains_key("unmappable_density"));
    assert!(
        doc_event
            .fields
            .contains_key("max_unmappable_density_per_doc")
    );
}

#[derive(Debug)]
struct BuildOutputs {
    summary: GutenbergBuildSummary,
    manifest_bytes: Vec<u8>,
    train_bytes: Vec<u8>,
    val_bytes: Vec<u8>,
    quality_bytes: Vec<u8>,
}

fn build_with_fixture(
    fixture_path: PathBuf,
    output_root: &Path,
) -> Result<BuildOutputs, GutenbergBuildError> {
    build_with_options(
        GutenbergBuildOptions {
            fixture_path,
            manifest_path: output_root.join("gutenberg-manifest.json"),
            train_path: output_root.join("gutenberg-train.bin"),
            val_path: output_root.join("gutenberg-val.bin"),
            corpus_quality_path: Some(output_root.join("corpus-quality.json")),
            tinystories_manifest_path: Some(
                workspace_root().join("fixtures/corpora/tinystories.toml"),
            ),
        },
        output_root,
    )
}

fn build_with_options(
    options: GutenbergBuildOptions,
    output_root: &Path,
) -> Result<BuildOutputs, GutenbergBuildError> {
    let summary = build_gutenberg_corpus(&options)?;
    let quality_path = options
        .corpus_quality_path
        .clone()
        .unwrap_or_else(|| output_root.join("corpus-quality.json"));
    Ok(BuildOutputs {
        summary,
        manifest_bytes: std::fs::read(&options.manifest_path).expect("manifest reads"),
        train_bytes: std::fs::read(&options.train_path).expect("train reads"),
        val_bytes: std::fs::read(&options.val_path).expect("val reads"),
        quality_bytes: std::fs::read(quality_path).expect("quality reads"),
    })
}

struct SyntheticBook {
    book_id: u32,
    body: String,
    marker_missing: bool,
}

impl SyntheticBook {
    fn new(book_id: u32, body: String) -> Self {
        Self {
            book_id,
            body,
            marker_missing: false,
        }
    }

    fn marker_missing(book_id: u32) -> Self {
        Self {
            book_id,
            body: "This source intentionally has no Gutenberg markers.".to_owned(),
            marker_missing: true,
        }
    }
}

fn write_synthetic_fixture(
    root: &Path,
    name: &str,
    retained_book_count_min: u32,
    books: &[SyntheticBook],
) -> PathBuf {
    let fixture_dir = root.join(format!("fixtures/corpora/{name}"));
    std::fs::create_dir_all(&fixture_dir).expect("synthetic fixture dir creates");
    let mut fixture = format!(
        "schema = \"gutenberg_smoke_fixture.v1\"\nsource_name = \"Project Gutenberg\"\nbook_count = {}\n\n[guards]\nretained_book_count_min = {retained_book_count_min}\n\n",
        books.len()
    );
    for book in books {
        let blob = if book.marker_missing {
            book.body.clone().into_bytes()
        } else {
            format!(
                "*** START OF THE PROJECT GUTENBERG EBOOK TEST ***\n{}\n*** END OF THE PROJECT GUTENBERG EBOOK TEST ***\n",
                book.body
            )
            .into_bytes()
        };
        let hash = sha256_hex_bytes(&blob);
        let relative_blob_path = format!(
            "fixtures/corpora/{name}/{}.{short}.bin",
            book.book_id,
            short = &hash[..16]
        );
        std::fs::write(root.join(&relative_blob_path), &blob).expect("synthetic blob writes");
        fixture.push_str(&format!(
            "[[sources]]\nbook_id = {}\ntitle = \"Synthetic {}\"\nauthor = \"Fixture\"\nsource_landing_url = \"https://www.gutenberg.org/ebooks/{}\"\nsource_blob_sha256 = \"{}\"\nsource_blob_size_bytes = {}\nmedia_type = \"text/plain; charset=utf-8\"\ncharset = \"utf-8\"\nlocal_blob_path = \"{}\"\n\n",
            book.book_id,
            book.book_id,
            book.book_id,
            hash,
            blob.len(),
            relative_blob_path
        ));
    }
    let fixture_path = root.join(format!("fixtures/corpora/{name}.toml"));
    std::fs::write(&fixture_path, fixture).expect("synthetic fixture writes");
    fixture_path
}

fn verify_smoke_expected(root: &Path) -> Result<(), String> {
    let expected = read_toml(root, SMOKE_EXPECTED)?;
    expect_string_eq(&expected, "schema", "gutenberg_smoke_expected.v1")?;
    expect_string_eq(&expected, "fixture_manifest_path", SMOKE_MANIFEST)?;
    expect_string_eq(
        &expected,
        "fixture_manifest_sha256",
        &sha256_uri_bytes(&std::fs::read(root.join(SMOKE_MANIFEST)).map_err(|error| {
            format!("{SMOKE_MANIFEST} reads for expected.toml verification: {error}")
        })?),
    )?;
    expect_string_eq(&expected, "gutenberg_manifest_self_hash", "pending:bd-29lv")?;
    expect_string_eq(&expected, "train_sha256", "pending:bd-29lv")?;
    expect_string_eq(&expected, "val_sha256", "pending:bd-29lv")?;

    let manifest = read_toml(root, SMOKE_MANIFEST)?;
    let manifest_sources = sources(&manifest);
    let expected_sources = array_field(&expected, "source_blobs")?;
    if expected_sources.len() != manifest_sources.len() {
        return Err(format!(
            "source_blobs length mismatch: expected.toml has {}, manifest has {}",
            expected_sources.len(),
            manifest_sources.len()
        ));
    }
    expect_integer_eq(
        &expected,
        "source_blob_count",
        i64::try_from(manifest_sources.len()).expect("source count fits i64"),
    )?;

    let mut retained_book_count = 0_i64;
    let mut total_fixture_bytes = 0_i64;
    let mut drop_count_source_decode_failed = 0_i64;
    let mut drop_count_invalid_utf8 = 0_i64;
    let mut drop_count_empty_after_strip = 0_i64;
    let mut drop_count_marker_missing = 0_i64;

    for (source, expected_source) in manifest_sources.iter().zip(expected_sources) {
        let book_id = integer_field(source, "book_id");
        expect_integer_eq(expected_source, "book_id", book_id)?;
        expect_string_eq(
            expected_source,
            "local_blob_path",
            &string_field(source, "local_blob_path"),
        )?;
        expect_string_eq(
            expected_source,
            "source_blob_sha256",
            &format!("sha256:{}", string_field(source, "source_blob_sha256")),
        )?;
        expect_integer_eq(
            expected_source,
            "source_blob_size_bytes",
            integer_field(source, "source_blob_size_bytes"),
        )?;

        let local_blob_path = string_field(source, "local_blob_path");
        let blob = std::fs::read(root.join(&local_blob_path))
            .map_err(|error| format!("{local_blob_path} reads for expected.toml: {error}"))?;
        let observed_blob_hash = sha256_uri_bytes(&blob);
        let expected_blob_hash = string_field_result(expected_source, "source_blob_sha256")?;
        if observed_blob_hash != expected_blob_hash {
            return Err(format!(
                "source blob sha256 drift for book {book_id}: expected {expected_blob_hash}, observed {observed_blob_hash}"
            ));
        }
        total_fixture_bytes += i64::try_from(blob.len()).expect("fixture byte length fits i64");

        match strip_gutenberg_d3(&blob) {
            Ok(stripped) if stripped.body.is_empty() => {
                drop_count_empty_after_strip += 1;
            }
            Ok(_) => {
                retained_book_count += 1;
            }
            Err(GutenbergD3DropReason::SourceDecodeFailed) => {
                drop_count_source_decode_failed += 1;
            }
            Err(GutenbergD3DropReason::InvalidUtf8) => {
                drop_count_invalid_utf8 += 1;
            }
            Err(GutenbergD3DropReason::GutenbergMarkerMissing) => {
                drop_count_marker_missing += 1;
            }
            Err(GutenbergD3DropReason::EmptyAfterStrip) => {
                drop_count_empty_after_strip += 1;
            }
        }
    }

    expect_integer_eq(&expected, "retained_book_count", retained_book_count)?;
    expect_integer_eq(&expected, "total_fixture_bytes", total_fixture_bytes)?;
    expect_integer_eq(
        &expected,
        "drop_count_total",
        drop_count_source_decode_failed
            + drop_count_invalid_utf8
            + drop_count_empty_after_strip
            + drop_count_marker_missing,
    )?;
    expect_integer_eq(&expected, "drop_count_no_supported_plaintext_format", 0)?;
    expect_integer_eq(&expected, "drop_count_no_plaintext_archive_member", 0)?;
    expect_integer_eq(
        &expected,
        "drop_count_source_decode_failed",
        drop_count_source_decode_failed,
    )?;
    expect_integer_eq(&expected, "drop_count_ambiguous_plaintext_archive", 0)?;
    expect_integer_eq(
        &expected,
        "drop_count_invalid_utf8",
        drop_count_invalid_utf8,
    )?;
    expect_integer_eq(
        &expected,
        "drop_count_empty_after_strip",
        drop_count_empty_after_strip,
    )?;
    expect_integer_eq(
        &expected,
        "drop_count_marker_missing",
        drop_count_marker_missing,
    )?;
    expect_integer_eq(&expected, "drop_count_unmappable_density", 0)?;
    expect_integer_eq(&expected, "drop_count_dedup_collision", 0)?;

    let owners = table_field(&expected, "owners")?;
    expect_string_eq(owners, "build_corpus_manifest_train_val", "bd-29lv")?;
    expect_string_eq(owners, "d3_strip_marker_drops", "bd-3vae")?;
    expect_string_eq(owners, "charset_unmappable_empty_drops", "bd-bzx3")?;
    expect_string_eq(owners, "contamination_report", "bd-2p3n")?;
    expect_string_eq(owners, "kn5_baseline", "bd-2nca")?;
    expect_string_eq(owners, "smoke_pipeline_logging", "bd-u6tn")?;

    Ok(())
}

fn copy_smoke_fixture_to_temp(root: &Path) -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("temp dir creates");
    copy_fixture_file(root, temp.path(), SMOKE_MANIFEST);
    copy_fixture_file(root, temp.path(), SMOKE_EXPECTED);

    let manifest = read_manifest(root);
    for source in sources(&manifest) {
        copy_fixture_file(root, temp.path(), &string_field(source, "local_blob_path"));
    }

    temp
}

fn copy_fixture_file(root: &Path, temp_root: &Path, relative_path: &str) {
    let source = root.join(relative_path);
    let destination = temp_root.join(relative_path);
    std::fs::create_dir_all(destination.parent().expect("fixture file has parent"))
        .expect("temp fixture parent creates");
    std::fs::copy(&source, &destination).unwrap_or_else(|error| {
        panic!(
            "copy fixture {} to temp {}: {error}",
            source.display(),
            destination.display()
        )
    });
}

fn read_manifest(root: &Path) -> Value {
    read_toml(root, SMOKE_MANIFEST).expect("smoke manifest parses as TOML")
}

fn read_toml(root: &Path, relative_path: &str) -> Result<Value, String> {
    let text = std::fs::read_to_string(root.join(relative_path))
        .map_err(|error| format!("{relative_path} reads: {error}"))?;
    toml::from_str(&text).map_err(|error| format!("{relative_path} parses as TOML: {error}"))
}

fn sources(manifest: &Value) -> &[Value] {
    manifest
        .get("sources")
        .and_then(Value::as_array)
        .expect("sources array")
}

fn array_field<'a>(value: &'a Value, field: &str) -> Result<&'a [Value], String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| format!("{field} array field"))
}

fn table_field<'a>(value: &'a Value, field: &str) -> Result<&'a Value, String> {
    let field_value = value
        .get(field)
        .ok_or_else(|| format!("{field} table field"))?;
    field_value
        .as_table()
        .map(|_| field_value)
        .ok_or_else(|| format!("{field} table field"))
}

fn integer_field(value: &Value, field: &str) -> i64 {
    value
        .get(field)
        .and_then(Value::as_integer)
        .unwrap_or_else(|| panic!("{field} integer field"))
}

fn string_field(value: &Value, field: &str) -> String {
    value
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{field} string field"))
        .to_owned()
}

fn string_field_result(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("{field} string field"))
}

fn expect_string_eq(value: &Value, field: &str, expected: &str) -> Result<(), String> {
    let observed = string_field_result(value, field)?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "{field} mismatch: expected {expected:?}, observed {observed:?}"
        ))
    }
}

fn expect_integer_eq(value: &Value, field: &str, expected: i64) -> Result<(), String> {
    let observed = value
        .get(field)
        .and_then(Value::as_integer)
        .ok_or_else(|| format!("{field} integer field"))?;
    if observed == expected {
        Ok(())
    } else {
        Err(format!(
            "{field} mismatch: expected {expected}, observed {observed}"
        ))
    }
}

fn assert_lower_sha256_hex(value: &str) {
    assert_eq!(value.len(), 64);
    assert!(
        value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte)),
        "sha256 pin must be lowercase hex"
    );
}

fn sha256_hex_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_uri_bytes(bytes: &[u8]) -> String {
    format!("sha256:{}", sha256_hex_bytes(bytes))
}

fn strip_utf8_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has a workspace parent")
        .to_path_buf()
}
