//! Byte-exact parity gate for the Rust BPE port against the frozen Python
//! conformance vectors (`training/gbtrain/conformance.py`).
//!
//! Loads the deployed `gbllm_bpe.v2` artifact and the frozen
//! `text <-> ids` vectors, then asserts, for every vector:
//!
//! 1. `encode(text) == golden ids`,
//! 2. `decode(ids) == text`, and
//! 3. round-trip `encode(decode(ids)) == ids`.
//!
//! Any divergence turns a silent inference-corruption bug into a loud one.

use std::path::PathBuf;

use gbf_data::bpe::BpeModel;
use serde::Deserialize;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-data has workspace parent")
        .to_path_buf()
}

#[derive(Debug, Deserialize)]
struct ConformanceFile {
    vocab_size: usize,
    vectors: Vec<ConformanceVector>,
}

#[derive(Debug, Deserialize)]
struct ConformanceVector {
    text: String,
    ids: Vec<u16>,
}

fn load_model() -> BpeModel {
    let path = workspace_root().join("training/artifacts/tinystories_bpe_1024.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read BPE artifact {}: {e}", path.display()));
    BpeModel::from_json(&text).expect("load + validate BPE artifact")
}

fn load_vectors() -> ConformanceFile {
    let path = workspace_root().join("training/artifacts/tinystories_bpe_1024.conformance.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read conformance vectors {}: {e}", path.display()));
    serde_json::from_str(&text).expect("parse conformance vectors")
}

#[test]
fn bpe_artifact_loads_and_reports_expected_shape() {
    let model = load_model();
    assert_eq!(model.vocab_size(), 1024, "deployed vocab is 1024 tokens");
    // Single-byte ids decode to their own byte.
    assert_eq!(model.id_bytes(97), Some(&b"a"[..]));
    // The header pins max_token_len = 11.
    assert_eq!(model.max_token_len(), 11);
}

#[test]
fn bpe_encode_matches_golden_conformance_vectors() {
    let model = load_model();
    let conformance = load_vectors();
    assert_eq!(
        conformance.vocab_size,
        model.vocab_size(),
        "conformance file vocab_size must match the loaded model"
    );

    let mut passed = 0usize;
    for vector in &conformance.vectors {
        // 1. Pre-tokenizer must be total (no dropped chars) — mirrors the
        //    Python conformance check.
        let joined: String = gbf_data::bpe::pretokenize(&vector.text).concat();
        assert_eq!(
            joined, vector.text,
            "pretokenizer not total on {:?}: dropped chars",
            vector.text
        );

        // 2. encode == golden ids.
        let ids = model.encode(&vector.text);
        assert_eq!(ids, vector.ids, "encode mismatch on {:?}", vector.text);

        // 3. decode == golden text.
        let decoded = model.decode(&ids);
        assert_eq!(decoded, vector.text, "decode mismatch on {:?}", vector.text);

        // 4. round-trip encode(decode(ids)) == ids.
        let round_trip = model.encode(&decoded);
        assert_eq!(
            round_trip, vector.ids,
            "round-trip encode(decode(ids)) mismatch on {:?}",
            vector.text
        );

        passed += 1;
    }

    assert_eq!(
        passed,
        conformance.vectors.len(),
        "every conformance vector must pass"
    );
    assert!(passed >= 15, "expected the frozen 15+ vector corpus");
}
