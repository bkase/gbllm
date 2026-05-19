use gbf_artifact::{BOS_ID, EOS_ID, ModelTokenSeq, RESERVED_ID, TextCharSeq, UNK_ID};
use proptest::prelude::*;

proptest! {
    #[test]
    fn valid_text_char_seq_always_constructs(ids in valid_text_char_seq()) {
        let seq = TextCharSeq::new(ids.clone()).expect("valid text ids construct");
        prop_assert_eq!(seq.as_slice(), ids.as_slice());
    }

    #[test]
    fn invalid_text_char_seq_always_rejects(ids in invalid_text_char_seq()) {
        prop_assert!(TextCharSeq::new(ids).is_err());
    }

    #[test]
    fn valid_model_token_seq_always_constructs(ids in valid_model_token_seq()) {
        let seq = ModelTokenSeq::new(ids.clone()).expect("valid model ids construct");
        prop_assert_eq!(seq.as_slice(), ids.as_slice());
    }

    #[test]
    fn invalid_model_token_seq_always_rejects(ids in invalid_model_token_seq()) {
        prop_assert!(ModelTokenSeq::new(ids).is_err());
    }
}

fn valid_text_char_seq() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![0u8..=75, Just(UNK_ID)], 0..64)
}

fn invalid_text_char_seq() -> impl Strategy<Value = Vec<u8>> {
    (
        valid_text_char_seq(),
        prop_oneof![Just(RESERVED_ID), Just(BOS_ID), Just(EOS_ID)],
        valid_text_char_seq(),
    )
        .prop_map(|(mut prefix, bad, suffix)| {
            prefix.push(bad);
            prefix.extend(suffix);
            prefix
        })
}

fn valid_model_token_seq() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(prop_oneof![0u8..=75, BOS_ID..=UNK_ID], 0..64)
}

fn invalid_model_token_seq() -> impl Strategy<Value = Vec<u8>> {
    (
        valid_model_token_seq(),
        prop_oneof![Just(RESERVED_ID), 80u8..=u8::MAX],
        valid_model_token_seq(),
    )
        .prop_map(|(mut prefix, bad, suffix)| {
            prefix.push(bad);
            prefix.extend(suffix);
            prefix
        })
}
