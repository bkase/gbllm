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
    assert_eq!(model.merges().len(), 768, "1024 = 256 base + 768 merges");
    assert_eq!(
        model.merges()[0],
        (32, 116),
        "ordered merge program remains artifact-exact"
    );
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

/// Cartridge-friendly formulation of the canonical merge loop: visit merge
/// ranks once, in artifact order, and replace every non-overlapping leftmost
/// occurrence within each pre-token chunk. A token created at rank `r` cannot
/// participate in an earlier merge (earlier merge operands were defined before
/// token `256 + r`), so no earlier rank ever needs revisiting.
fn encode_by_rank_passes(model: &BpeModel, text: &str) -> Vec<u16> {
    let mut out = Vec::new();
    for chunk in gbf_data::bpe::pretokenize(text) {
        let mut ids: Vec<u16> = chunk.bytes().map(u16::from).collect();
        for (rank, &(left, right)) in model.merges().iter().enumerate() {
            let mut pos = 0usize;
            while pos + 1 < ids.len() {
                if ids[pos] == left && ids[pos + 1] == right {
                    ids[pos] = (256 + rank) as u16;
                    ids.remove(pos + 1);
                }
                pos += 1;
            }
        }
        out.extend(ids);
    }
    out
}

#[test]
fn rank_ordered_merge_passes_equal_the_canonical_encoder() {
    let model = load_model();
    let mut cases = vec![
        String::new(),
        "Once upon a time".into(),
        "The machines dreamed".into(),
        "8\n TA\nz".into(),
        "two  spaces".into(),
        "A cat sat.".into(),
        "123 words!?".into(),
        "___ __".into(),
    ];

    // Exhaust a compact alphabet across the ASCII pre-tokenizer classes. This
    // catches accidental cross-chunk merges as well as equal-rank overlap.
    const ALPHABET: &[u8] = b"aA0 .!\n_";
    for &a in ALPHABET {
        for &b in ALPHABET {
            for &c in ALPHABET {
                cases.push(String::from_utf8(vec![a, b, c]).expect("ASCII"));
            }
        }
    }

    for text in cases {
        assert_eq!(
            encode_by_rank_passes(&model, &text),
            model.encode(&text),
            "rank-pass encoder diverged on {text:?}"
        );
    }
}
