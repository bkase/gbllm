#![cfg(feature = "s4")]

mod common;

use std::path::{Path, PathBuf};

use common::tracing_capture::{TraceCapture, captured_events, with_trace_capture};
use gbf_artifact::{BOS_ID, EOS_ID, GutenbergManifest, TextCharSeq};
use gbf_data::charset_v1::normalize_raw;
use gbf_experiments::s3::baseline::{
    KN_MAX_ORDER, KN_RESET_CHUNK_SIZE, KnBaselineInputs, s3_fit_kn5,
};
use gbf_experiments::s4::baseline::{
    S4_BASELINE_GUTENBERG_SCHEMA, S4BaselineError, S4BaselineInputs, s4_fit_kn5_gutenberg,
};
use gbf_experiments::s4::manifest::{GutenbergBuildOptions, build_gutenberg_corpus};
use gbf_foundation::{CanonicalJson, Hash256, sha256};
use serde_json::{Value as JsonValue, json};
use toml::Value as TomlValue;

const SMOKE_MANIFEST: &str = "fixtures/corpora/gutenberg_smoke.toml";
const SMOKE_EXPECTED: &str = "fixtures/corpora/gutenberg_smoke/expected.toml";

#[test]
fn s4_baseline_report_reuses_s3_kn5_outputs_and_self_hashes() {
    let (train, val) = oracle_sequences();
    let s3 = s3_fit_kn5(KnBaselineInputs {
        train_post: train.clone(),
        val_post: val.clone(),
    })
    .expect("S3 oracle baseline fits");

    let report = s4_fit_kn5_gutenberg(inputs(train, val)).expect("S4 baseline fits");

    assert_eq!(report.schema, S4_BASELINE_GUTENBERG_SCHEMA);
    assert_eq!(report.kn_params.max_order, KN_MAX_ORDER as u64);
    assert_eq!(
        report.kn_params.reset_context_chunk_size,
        KN_RESET_CHUNK_SIZE as u64
    );
    assert_eq!(report.kn_params.discounts, s3.discounts);
    assert_eq!(report.bpc_kn5, s3.bpc_kn5_val);
    assert_eq!(report.bpc_kn3, s3.bpc_kn3_val);
    assert_eq!(report.bpc_unigram, s3.bpc_kn1_val);
    assert_eq!(report.counts_summary, s3.counts_summary);
    assert_eq!(report.counts_blob_sha256, s3.counts_blob_sha256);
    assert_eq!(
        report.baseline_gutenberg_self_hash,
        report.compute_self_hash().expect("self-hash recomputes")
    );
    report
        .validate_canonical_write()
        .expect("canonical report validates");

    let emitted_json: JsonValue =
        serde_json::from_slice(&report.canonical_bytes().expect("canonical bytes"))
            .expect("canonical report parses");
    let canonical_bytes =
        CanonicalJson::value_to_vec(&emitted_json).expect("parse-normalized canonical bytes");
    let json: JsonValue = serde_json::from_slice(&canonical_bytes).unwrap();
    // bd-3gjy: pin the RFC §12.4 `s4_baseline_gutenberg.v1` field list.
    let json_fields = json
        .as_object()
        .expect("baseline report is object")
        .keys()
        .map(String::as_str)
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        json_fields,
        [
            "baseline_gutenberg_self_hash",
            "bpc_kn3",
            "bpc_kn5",
            "bpc_unigram",
            "corpus_train_sha",
            "corpus_val_sha",
            "counts_blob_sha256",
            "counts_summary",
            "gutenberg_manifest_self_hash",
            "kn_params",
            "schema",
            "tinystories_manifest_self_hash",
        ]
        .into_iter()
        .collect()
    );
    assert_eq!(json["schema"], json!("s4_baseline_gutenberg.v1"));
    assert_eq!(
        json["kn_params"]["smoothing"],
        json!("modified_kneser_ney_d_rule")
    );
    assert_eq!(json["kn_params"]["max_order"], json!(5));
    assert_eq!(json["kn_params"]["reset_context_chunk_size"], json!(128));
    let discounts = json["kn_params"]["discounts"]
        .as_object()
        .expect("discounts object");
    assert_eq!(
        discounts.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["2", "3", "4", "5"]
    );
    assert_eq!(
        json["kn_params"]["discounts"]["2"]["d_1"],
        json!(report.kn_params.discounts[&2].d_1)
    );
    assert!(
        json["bpc_kn5"]
            .as_f64()
            .expect("bpc_kn5 numeric")
            .is_finite()
    );
    assert!(json["counts_blob_sha256"].as_str().is_some());
    assert!(json["baseline_gutenberg_self_hash"].as_str().is_some());
    assert_eq!(
        CanonicalJson::value_to_vec(&json).expect("parsed JSON recanonicalizes"),
        canonical_bytes
    );
}

#[test]
fn s4_baseline_rejects_caller_split_hash_mismatch_before_manifest_binding() {
    let (train, val) = oracle_sequences();
    let mut inputs = inputs(train, val);
    inputs.corpus_val_sha = hash(99);

    let error = s4_fit_kn5_gutenberg(inputs).expect_err("hash mismatch rejected");

    assert!(matches!(
        error,
        S4BaselineError::CorpusHashMismatch {
            field: "corpus_val_sha",
            ..
        }
    ));
}

#[test]
fn s4_baseline_two_cold_fits_are_byte_identical() {
    let first = s4_fit_kn5_gutenberg(oracle_inputs()).expect("first cold fit succeeds");
    let second = s4_fit_kn5_gutenberg(oracle_inputs()).expect("second cold fit succeeds");

    assert_eq!(first, second);
    assert_eq!(
        first.canonical_bytes().expect("first canonicalizes"),
        second.canonical_bytes().expect("second canonicalizes")
    );
}

#[test]
fn s4_baseline_gutenberg_smoke_fixture_matches_expected_bpc() {
    let root = workspace_root();
    let expected = read_toml(&root, SMOKE_EXPECTED);
    let baseline_expected = table_field(&expected, "kn5_baseline");
    assert_eq!(
        string_field(baseline_expected, "owner_bead"),
        "bd-2nca",
        "expected.toml should route the smoke baseline pin to the baseline bead"
    );
    assert_eq!(
        string_field(baseline_expected, "manifest_binding_owner"),
        "bd-29lv",
        "real manifest train/val hash binding remains with build-corpus ownership"
    );
    let expected_bpc = float_field(baseline_expected, "expected_bpc_kn5");
    let tolerance = float_field(baseline_expected, "tolerance");

    let report = fit_smoke_baseline(&root);
    report
        .validate_canonical_write()
        .expect("smoke baseline canonical write validates");
    let delta = (report.bpc_kn5 - expected_bpc).abs();
    assert!(
        delta <= tolerance,
        "smoke fixture KN-5 bpc drift: observed {:.17}, expected {:.17} ± {:.3e}",
        report.bpc_kn5,
        expected_bpc,
        tolerance
    );
}

#[test]
fn s4_baseline_validation_rejects_nonfinite_bpc() {
    let (train, val) = oracle_sequences();
    let mut report = s4_fit_kn5_gutenberg(inputs(train, val)).expect("S4 baseline fits");
    report.bpc_kn5 = f64::INFINITY;
    report.baseline_gutenberg_self_hash = report.compute_self_hash().expect("self hash updates");

    let error = report
        .validate_canonical_write()
        .expect_err("nonfinite bpc rejected");

    assert!(matches!(
        error,
        S4BaselineError::NonFiniteOrNegative {
            field: "bpc_kn5",
            ..
        }
    ));
}

#[test]
fn s4_baseline_logging_emits_artifact_hash() {
    let (train, val) = oracle_sequences();
    let capture = TraceCapture::default();

    let report = with_trace_capture(&capture, || {
        s4_fit_kn5_gutenberg(inputs(train, val)).expect("S4 baseline fits")
    });

    let events = captured_events(&capture);
    assert!(
        events
            .iter()
            .any(|event| event.name == "s4::baseline::fit_started")
    );
    let emitted = events
        .iter()
        .find(|event| event.name == "s4::baseline::emitted")
        .expect("baseline emitted event");
    assert_eq!(
        emitted.fields.get("baseline_gutenberg_self_hash"),
        Some(&json!(report.baseline_gutenberg_self_hash.to_string()))
    );
    assert_eq!(
        emitted.fields.get("counts_blob_sha256"),
        Some(&json!(report.counts_blob_sha256.to_string()))
    );
}

fn inputs(train: TextCharSeq, val: TextCharSeq) -> S4BaselineInputs {
    S4BaselineInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: hash(2),
        corpus_train_sha: sha256(train.as_slice()),
        corpus_val_sha: sha256(val.as_slice()),
        corpus_train: train,
        corpus_val: val,
    }
}

fn oracle_inputs() -> S4BaselineInputs {
    let (train, val) = oracle_sequences();
    inputs(train, val)
}

fn oracle_sequences() -> (TextCharSeq, TextCharSeq) {
    let root = workspace_root().join("fixtures/baselines/kn_oracle");
    (
        normalize_file(&root.join("train.bytes")),
        normalize_file(&root.join("eval.bytes")),
    )
}

fn normalize_file(path: &Path) -> TextCharSeq {
    let bytes = std::fs::read(path).expect("fixture bytes read");
    let normalized = normalize_raw(&bytes).expect("fixture normalizes");
    assert!(!normalized.dropped);
    normalized.tokens
}

fn fit_smoke_baseline(root: &Path) -> gbf_experiments::s4::baseline::S4BaselineGutenbergReport {
    let temp = tempfile::tempdir().expect("tempdir");
    let manifest_path = temp.path().join("gutenberg-manifest.json");
    let train_path = temp.path().join("gutenberg-train.bin");
    let val_path = temp.path().join("gutenberg-val.bin");
    let summary = build_gutenberg_corpus(&GutenbergBuildOptions {
        fixture_path: root.join(SMOKE_MANIFEST),
        manifest_path: manifest_path.clone(),
        train_path: train_path.clone(),
        val_path: val_path.clone(),
        corpus_quality_path: None,
        tinystories_manifest_path: None,
    })
    .expect("smoke fixture builds corpus for baseline");
    let manifest_bytes = std::fs::read(&manifest_path).expect("smoke manifest reads");
    let manifest: GutenbergManifest =
        serde_json::from_slice(&manifest_bytes).expect("smoke manifest parses");
    assert_eq!(manifest.manifest_self_hash, summary.manifest_self_hash);

    let train = baseline_text_seq(std::fs::read(&train_path).expect("smoke train reads"));
    let val = baseline_text_seq(std::fs::read(&val_path).expect("smoke val reads"));
    s4_fit_kn5_gutenberg(S4BaselineInputs {
        tinystories_manifest_self_hash: hash(1),
        gutenberg_manifest_self_hash: manifest.manifest_self_hash,
        corpus_train_sha: sha256(train.as_slice()),
        corpus_val_sha: sha256(val.as_slice()),
        corpus_train: train,
        corpus_val: val,
    })
    .expect("smoke baseline fits")
}

fn baseline_text_seq(bytes: Vec<u8>) -> TextCharSeq {
    let ids = bytes
        .into_iter()
        .filter(|id| *id != BOS_ID && *id != EOS_ID)
        .collect();
    TextCharSeq::new(ids).expect("build-corpus text ids are valid after boundary stripping")
}

fn read_toml(root: &Path, relative_path: &str) -> TomlValue {
    let text = std::fs::read_to_string(root.join(relative_path)).unwrap_or_else(|error| {
        panic!("{relative_path} reads: {error}");
    });
    toml::from_str(&text).unwrap_or_else(|error| panic!("{relative_path} parses: {error}"))
}

fn table_field<'a>(value: &'a TomlValue, field: &str) -> &'a TomlValue {
    let field_value = value.get(field).unwrap_or_else(|| panic!("{field} table"));
    field_value
        .as_table()
        .map(|_| field_value)
        .unwrap_or_else(|| panic!("{field} table"))
}

fn string_field(value: &TomlValue, field: &str) -> String {
    value
        .get(field)
        .and_then(TomlValue::as_str)
        .unwrap_or_else(|| panic!("{field} string"))
        .to_owned()
}

fn float_field(value: &TomlValue, field: &str) -> f64 {
    value
        .get(field)
        .and_then(TomlValue::as_float)
        .unwrap_or_else(|| panic!("{field} float"))
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn hash(fill: u8) -> Hash256 {
    Hash256::from_bytes([fill; 32])
}
