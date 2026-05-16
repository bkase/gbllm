use gbf_artifact::UNK_ID;
use gbf_data::charset_v1::{CharsetInputs, s3_charset_v1};

#[test]
fn charset_endoftext_literal_is_source_text_not_example_boundary() {
    let stats = gbf_data::charset_v1::normalize_raw(b"<|endoftext|>").unwrap();
    let ids = stats.tokens.as_slice();

    assert_eq!(ids.len(), 13);
    assert_eq!(ids[0], UNK_ID);
    assert_eq!(ids[1], UNK_ID);
    assert_eq!(&ids[2..11], &[30, 39, 29, 40, 31, 45, 30, 49, 45]);
    assert_eq!(ids[11], UNK_ID);
    assert_eq!(ids[12], UNK_ID);

    let product = s3_charset_v1(CharsetInputs {
        raw_train_examples: vec![b"<|endoftext|>".to_vec()],
        raw_val_examples: vec![],
        spec: gbf_artifact::LexicalSpec_v1::pinned(),
    })
    .expect("pipeline handles literal endoftext as one source example");

    assert_eq!(product.drop_log.len(), 1);
    assert_eq!(product.drop_log[0].example_id, 0);
    assert!(product.train_post.is_empty());
}
