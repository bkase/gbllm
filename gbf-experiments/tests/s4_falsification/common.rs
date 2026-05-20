#![allow(dead_code)]

use std::collections::BTreeMap;

use gbf_artifact::{BOS_ID, EOS_ID, GutenbergManifest, GutenbergSplit, UNK_ID};
use gbf_experiments::s4::corpus_oracle::{
    S4ContaminationMathFixture, S4CorpusOracleInputs, S4ForcedIndexCollision,
    S4SplitDeterminismFixture, S4StripperOracleCase, contamination_math_fixture,
    fixture_post_strip_sha256,
};
use gbf_experiments::s4::falsify::{
    S4FalsificationSuiteInputs, S4LineageFalsificationFixture, S4OracleDriftFalsificationFixture,
    S4PromotionGateFalsificationFixture,
};
use gbf_foundation::Hash256;

pub fn suite_inputs() -> S4FalsificationSuiteInputs {
    S4FalsificationSuiteInputs {
        corpus_oracle_inputs: clean_corpus_oracle_inputs(),
        promotion_gate: S4PromotionGateFalsificationFixture {
            oracle_agreement_required: true,
            oracle_agreement_artifact_present: false,
            oracle_agreement_agrees: false,
            broken_gate_promotes_without_oracle: true,
        },
        lineage: S4LineageFalsificationFixture {
            c_ts_checkpoint_payload_sha: hash(0x11),
            actual_initial_checkpoint_payload_sha: hash(0x22),
            recorded_initial_checkpoint_payload_sha: hash(0x11),
        },
        oracle_drift: S4OracleDriftFalsificationFixture {
            expected_artifact_oracle_corpus: "gutenberg_val",
            observed_artifact_oracle_corpus: "tinystories_val",
            live_training_bpc: 1.25,
            reference_bundle_bpc: 1.25,
            artifact_oracle_bpc: 1.25,
            tolerance: 1.0e-6,
        },
    }
}

pub fn clean_corpus_oracle_inputs() -> S4CorpusOracleInputs {
    let manifest_json =
        include_str!("../../../fixtures/schemas/s4/gutenberg_manifest_v1_minimal.json")
            .trim_end()
            .as_bytes()
            .to_vec();
    let manifest: GutenbergManifest =
        serde_json::from_slice(&manifest_json).expect("minimal manifest fixture parses");
    let raw_utf8 = gutenberg_raw("Café body with ASCII punctuation!\n");
    let split_map = split_map(&manifest);
    let train_bytes = b"train-bytes-replayed".to_vec();
    let val_bytes = b"val-bytes-replayed".to_vec();

    S4CorpusOracleInputs {
        manifest: manifest.clone(),
        manifest_canonical_json: manifest_json,
        stripper_cases: vec![S4StripperOracleCase {
            expected_post_strip_sha256: fixture_post_strip_sha256(&raw_utf8)
                .expect("fixture strips"),
            raw_utf8,
        }],
        charset_roundtrip_prefix: vec![BOS_ID, 0, 26, 75, UNK_ID, EOS_ID],
        split_replay: S4SplitDeterminismFixture {
            expected_split_map: split_map.clone(),
            replayed_split_map: split_map,
            expected_train_bytes: train_bytes.clone(),
            replayed_train_bytes: train_bytes,
            expected_val_bytes: val_bytes.clone(),
            replayed_val_bytes: val_bytes,
        },
        unmappable_manifest: manifest,
        contamination_math: clean_contamination_fixture(),
    }
}

pub fn clean_contamination_fixture() -> S4ContaminationMathFixture {
    let shared = window(1);
    let left_only = window(2);
    let right_only = window(3);
    let collision_left = window(20);
    let collision_right = window(21);
    contamination_math_fixture(
        vec![shared, left_only, collision_left],
        vec![shared, right_only, collision_right],
        1,
        vec![S4ForcedIndexCollision {
            left_window: collision_left,
            right_window: collision_right,
            forced_index: 7,
        }],
    )
}

pub fn split_map(manifest: &GutenbergManifest) -> BTreeMap<u32, GutenbergSplit> {
    manifest
        .sources
        .iter()
        .filter_map(|source| source.split.map(|split| (source.book_id, split)))
        .collect()
}

pub fn gutenberg_raw(body: &str) -> Vec<u8> {
    format!(
        "Header\n*** START OF THE PROJECT GUTENBERG EBOOK FIXTURE ***\n{body}\n*** END OF THE PROJECT GUTENBERG EBOOK FIXTURE ***\nFooter\n"
    )
    .into_bytes()
}

pub fn window(seed: u8) -> [u8; 13] {
    let mut out = [0_u8; 13];
    for (idx, byte) in out.iter_mut().enumerate() {
        *byte = seed.wrapping_add(idx as u8);
    }
    out
}

pub fn hash(byte: u8) -> Hash256 {
    Hash256::from_bytes([byte; 32])
}
