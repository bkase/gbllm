use gbf_data::charset_v1::{CharsetInputs, s3_charset_v1};

#[test]
fn charset_drop_rate_accounting_uses_examples_and_dropped_token_numerator() {
    let dropped = std::iter::repeat_n("😀".as_bytes().to_vec(), 5);
    let kept = std::iter::repeat_n(format!("{}😀", "a".repeat(49)).into_bytes(), 95);
    let raw_train_examples = dropped.chain(kept).collect::<Vec<_>>();

    let product = s3_charset_v1(CharsetInputs {
        raw_train_examples,
        raw_val_examples: vec![],
        spec: gbf_artifact::LexicalSpec_v1::pinned(),
    })
    .expect("drop-rate fixture normalizes");

    assert_eq!(product.drop_log.len(), 5);
    assert_eq!(product.unmappable_example_drop_rate_train, 0.05);

    let expected_dropped_tokens = 5.0;
    let expected_pre_drop_tokens = expected_dropped_tokens + 95.0 * 50.0;
    assert_eq!(
        product.unmappable_char_drop_rate_train,
        expected_dropped_tokens / expected_pre_drop_tokens
    );
}
