use gbf_artifact::{
    BOS_ID, EOS_ID, LexicalError, ModelTokenSeq, RESERVED_ID, TextCharSeq, UNK_ID, is_control_id,
    is_printable_id, is_text_char_id,
};

#[test]
fn charset_v1_predicates_pin_boundaries() {
    assert!(is_printable_id(0));
    assert!(is_printable_id(75));
    assert!(!is_printable_id(RESERVED_ID));

    assert!(!is_control_id(75));
    assert!(is_control_id(BOS_ID));
    assert!(is_control_id(EOS_ID));
    assert!(is_control_id(UNK_ID));

    assert!(is_text_char_id(0));
    assert!(is_text_char_id(75));
    assert!(is_text_char_id(UNK_ID));
    assert!(!is_text_char_id(RESERVED_ID));
    assert!(!is_text_char_id(BOS_ID));
    assert!(!is_text_char_id(EOS_ID));
}

#[test]
fn text_char_seq_rejects_every_forbidden_id_at_start_middle_and_end() {
    for id in forbidden_text_ids() {
        for position in [0, 1, 2] {
            let ids = ids_with_bad_at(position, id);
            let err = TextCharSeq::new(ids).expect_err("forbidden text id rejects");
            assert_eq!(
                err,
                expected_text_error(id, position),
                "id {id} at position {position} returned wrong error"
            );
        }
    }
}

#[test]
fn model_token_seq_rejects_reserved_and_out_of_range_ids_at_start_middle_and_end() {
    for id in forbidden_model_ids() {
        for position in [0, 1, 2] {
            let ids = ids_with_bad_at(position, id);
            let err = ModelTokenSeq::new(ids).expect_err("forbidden model id rejects");
            assert_eq!(
                err,
                expected_model_error(id, position),
                "id {id} at position {position} returned wrong error"
            );
        }
    }
}

#[test]
fn checked_sequences_preserve_valid_ids() {
    let text = TextCharSeq::new(vec![0, 62, 75, UNK_ID]).expect("valid text ids");
    assert_eq!(text.as_slice(), &[0, 62, 75, UNK_ID]);
    assert_eq!(text.into_vec(), vec![0, 62, 75, UNK_ID]);

    let model =
        ModelTokenSeq::new(vec![BOS_ID, 0, 62, 75, UNK_ID, EOS_ID]).expect("valid model ids");
    assert_eq!(model.as_slice(), &[BOS_ID, 0, 62, 75, UNK_ID, EOS_ID]);
}

#[test]
fn checked_sequences_reject_reserved_and_out_of_range_ids_on_deserialize() {
    for json in ["[76]", "[80]"] {
        serde_json::from_str::<TextCharSeq>(json)
            .expect_err("text sequence rejects invalid deserialized id");
        serde_json::from_str::<ModelTokenSeq>(json)
            .expect_err("model token sequence rejects invalid deserialized id");
    }
}

fn forbidden_text_ids() -> Vec<u8> {
    let mut ids = vec![RESERVED_ID, BOS_ID, EOS_ID];
    ids.extend(80..=u8::MAX);
    ids
}

fn forbidden_model_ids() -> Vec<u8> {
    let mut ids = vec![RESERVED_ID];
    ids.extend(80..=u8::MAX);
    ids
}

fn ids_with_bad_at(position: usize, id: u8) -> Vec<u8> {
    let mut ids = vec![0, 1, 2];
    ids[position] = id;
    ids
}

fn expected_text_error(id: u8, position: usize) -> LexicalError {
    match id {
        RESERVED_ID => LexicalError::ReservedId76 { position },
        BOS_ID | EOS_ID => LexicalError::ControlIdInTextStream { id, position },
        _ => LexicalError::OutOfRange { id, position },
    }
}

fn expected_model_error(id: u8, position: usize) -> LexicalError {
    match id {
        RESERVED_ID => LexicalError::ReservedId76 { position },
        _ => LexicalError::OutOfRange { id, position },
    }
}
