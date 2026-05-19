use gbf_artifact::{LexicalSpec_v1, NormalizationSpec_v1};

#[test]
fn lexical_self_hash_is_deterministic_across_replays() {
    let first = LexicalSpec_v1::pinned().lexical_self_hash;
    for _ in 0..10 {
        let spec = LexicalSpec_v1::pinned();
        assert_eq!(spec.lexical_self_hash, first);
        assert_eq!(spec.compute_self_hash(), first);
    }
}

#[test]
fn normalization_self_hash_is_deterministic_across_replays() {
    let first = NormalizationSpec_v1::pinned().normalization_self_hash;
    for _ in 0..10 {
        let spec = NormalizationSpec_v1::pinned();
        assert_eq!(spec.normalization_self_hash, first);
        assert_eq!(spec.compute_self_hash(), first);
    }
}
