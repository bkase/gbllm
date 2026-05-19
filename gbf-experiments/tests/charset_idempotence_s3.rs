use std::path::PathBuf;

use gbf_artifact::{BOS_ID, EOS_ID, RESERVED_ID, UNK_ID};
use gbf_data::charset_v1::{normalize_raw, normalize_tokens};

#[test]
fn charset_idempotence_fixtures_round_trip_through_token_normalizer() {
    let fixture_dir = fixture_dir();
    let mut entries = std::fs::read_dir(&fixture_dir)
        .expect("charset fixture dir exists")
        .map(|entry| entry.expect("fixture dir entry").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("txt"))
        .collect::<Vec<_>>();
    entries.sort();

    assert!(!entries.is_empty(), "idempotence fixtures must be present");
    let mut saw_unk_second_pass_fixture = false;

    for path in entries {
        let bytes = std::fs::read(&path).expect("fixture reads");
        let stats = normalize_raw(&bytes).expect("fixture normalizes");
        let normalized_again = normalize_tokens(stats.tokens.clone());

        assert_eq!(
            normalized_again,
            stats.tokens,
            "normalize_tokens must be idempotent for {}",
            path.display()
        );
        assert!(
            stats
                .tokens
                .as_slice()
                .iter()
                .all(|id| matches!(*id, 0..=75 | UNK_ID)),
            "fixture {} emitted an invalid corpus id: {:?}",
            path.display(),
            stats.tokens.as_slice()
        );
        assert!(!stats.tokens.as_slice().contains(&RESERVED_ID));
        assert!(!stats.tokens.as_slice().contains(&BOS_ID));
        assert!(!stats.tokens.as_slice().contains(&EOS_ID));

        if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("unk_second_pass"))
        {
            assert!(
                stats.tokens.as_slice().contains(&UNK_ID),
                "unk_second_pass fixture must emit id-79 on first pass"
            );
            assert!(
                !stats.dropped,
                "unk_second_pass fixture should exercise kept already-tokenized <unk>"
            );
            saw_unk_second_pass_fixture = true;
        }
    }

    assert!(
        saw_unk_second_pass_fixture,
        "fixture set must contain an unk_second_pass example"
    );
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-experiments has workspace parent")
        .join("fixtures/corpora/charset_v1_idempotence")
}
