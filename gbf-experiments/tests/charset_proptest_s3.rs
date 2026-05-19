use gbf_artifact::{TextCharSeq, UNK_ID};
use gbf_data::charset_v1::{
    TextCharSeqWithStats, decide_drop, normalize_raw, normalize_tokens, unk_fraction,
};
use proptest::prelude::*;

proptest! {
    #[test]
    fn charset_normalize_raw_valid_utf8_never_panics(input in any::<String>()) {
        let _ = normalize_raw(input.as_bytes());
    }

    #[test]
    fn charset_normalize_tokens_is_idempotent(ids in text_char_ids()) {
        let seq = TextCharSeq::new(ids).expect("strategy emits valid text ids");
        let once = normalize_tokens(seq.clone());
        let twice = normalize_tokens(once.clone());
        prop_assert_eq!(once.clone(), seq);
        prop_assert_eq!(twice, once);
    }

    #[test]
    fn charset_drop_decision_matches_strict_two_percent_threshold(total in 1usize..=200, unk in 0usize..=200) {
        let unk = unk.min(total);
        let mut ids = vec![0; total - unk];
        ids.extend(std::iter::repeat_n(UNK_ID, unk));
        let stats = TextCharSeqWithStats {
            tokens: TextCharSeq::new(ids).expect("valid ids"),
            unk_count_in_example: unk as u32,
            dropped: false,
            drop_reason: None,
        };
        let fraction = unk_fraction(&stats);

        prop_assert_eq!(decide_drop(fraction), fraction > 0.02);
    }
}

fn text_char_ids() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![0u8..=75, Just(UNK_ID)], 0..256)
}
