use gbf_artifact::UNK_ID;
use gbf_data::charset_v1::{
    collapse_internal_ascii_spaces, encode_charset_v1, fold_quotes_dashes_ellipsis,
    normalize_line_endings, normalize_raw, normalize_whitespace, strip_combining_accents,
    tabs_to_spaces, trim_trailing_ascii_spaces, unicode_nfc,
};

#[test]
fn charset_nfc_composed_and_decomposed_match() {
    assert_eq!(unicode_nfc("e\u{301}"), "é");
    assert_eq!(
        normalize_raw("é".as_bytes()).unwrap().tokens,
        normalize_raw("e\u{301}".as_bytes()).unwrap().tokens
    );
}

#[test]
fn charset_strip_combining_accents_maps_accented_latin_to_ascii_base() {
    assert_eq!(strip_combining_accents("é"), "e");
    assert_eq!(strip_combining_accents("e\u{301}"), "e");
    assert_eq!(
        normalize_raw("Noël".as_bytes()).unwrap().tokens,
        normalize_raw("Noel".as_bytes()).unwrap().tokens
    );
}

#[test]
fn charset_preserves_case() {
    let upper = normalize_raw("ABC".as_bytes()).unwrap().tokens;
    let lower = normalize_raw("abc".as_bytes()).unwrap().tokens;
    assert_ne!(upper, lower);
}

#[test]
fn charset_folds_quotes_dashes_and_ellipsis() {
    assert_eq!(fold_quotes_dashes_ellipsis("“abc”"), "\"abc\"");
    assert_eq!(fold_quotes_dashes_ellipsis("«abc»"), "\"abc\"");
    assert_eq!(fold_quotes_dashes_ellipsis("a—b–c…"), "a--b-c...");
    assert_eq!(
        fold_quotes_dashes_ellipsis("\u{2010}\u{2011}\u{2012}\u{201B}\u{201E}\u{201F}\u{2212}"),
        "\u{2010}\u{2011}\u{2012}\u{201B}\u{201E}\u{201F}\u{2212}"
    );
}

#[test]
fn charset_whitespace_substeps_are_ordered_and_observable() {
    assert_eq!(normalize_line_endings("a\r\nb\rc"), "a\nb\nc");
    assert_eq!(tabs_to_spaces("a\tb"), "a b");
    assert_eq!(trim_trailing_ascii_spaces("a  \n b   "), "a\n b");
    assert_eq!(collapse_internal_ascii_spaces("a   b\n  c"), "a b\n c");
    assert_eq!(normalize_whitespace("a\t \r\nb   \r c"), "a\nb\n c");
}

#[test]
fn charset_unmappable_maps_to_unk_and_drop_decision_counts() {
    let stats = normalize_raw("Ж漢😀".as_bytes()).unwrap();
    assert_eq!(stats.tokens.as_slice(), &[UNK_ID, UNK_ID, UNK_ID]);
    assert_eq!(stats.unk_count_in_example, 3);
    assert!(stats.dropped);

    let (ids, unk_count) = encode_charset_v1("<|>");
    assert_eq!(ids, vec![UNK_ID, UNK_ID, UNK_ID]);
    assert_eq!(unk_count, 3);
}
