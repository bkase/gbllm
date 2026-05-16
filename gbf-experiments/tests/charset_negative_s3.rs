use gbf_artifact::LexicalSpec_v1;
use gbf_artifact::RESERVED_ID;
use gbf_data::charset_v1::{
    CharsetError, CharsetInputs, CharsetProduct, normalize_raw, normalize_token_ids, s3_charset_v1,
    verify_charseq_sha256,
};
use gbf_foundation::Hash256;

#[test]
fn charset_reserved_id_76_in_token_input_rejects_before_loader_allocation() {
    let err = normalize_token_ids(vec![0, RESERVED_ID, 1]).expect_err("reserved id rejects");

    assert!(matches!(
        err,
        CharsetError::ReservedIdInInput { position: 1 }
    ));
}

#[test]
fn charset_post_sha_mismatch_rejects_before_scoring() {
    let seq = normalize_raw(b"abc").expect("normalizes").tokens;
    let expected = Hash256::from_bytes([7; 32]);
    let err = verify_charseq_sha256(&seq, expected).expect_err("wrong sha rejects");

    assert!(matches!(
        err,
        CharsetError::PostShaMismatch {
            expected: observed_expected,
            ..
        } if observed_expected == expected
    ));
}

#[test]
fn charset_product_schema_literal_is_pinned_on_deserialize() {
    let product = s3_charset_v1(CharsetInputs {
        raw_train_examples: vec![b"abc".to_vec()],
        raw_val_examples: Vec::new(),
        spec: LexicalSpec_v1::pinned(),
    })
    .expect("valid product");
    let mut value = serde_json::to_value(product).expect("serializes");
    value["schema"] = serde_json::json!("wrong.v1");

    serde_json::from_value::<CharsetProduct>(value).expect_err("wrong schema rejects");
}
