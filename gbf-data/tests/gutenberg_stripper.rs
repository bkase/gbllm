use std::path::{Path, PathBuf};

use gbf_data::{
    GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN, GUTENBERG_D3_HEADER_REGEX_PATTERN,
    GutenbergD3DropReason, empty_after_strip_reason, marker_missing_drop_cap_breached,
    normalize_gutenberg_d3_text, strip_gutenberg_d3,
};
use serde::Deserialize;

#[test]
fn golden_gutenberg_strip_fixture_matches_expected_bodies_and_drops() {
    let root = workspace_root();
    let fixture = read_fixture(&root);
    assert_eq!(fixture.schema, "gutenberg_strip_golden_expected.v1");
    assert_eq!(fixture.case_count, 10);
    assert_eq!(fixture.cases.len(), 10);

    for case in fixture.cases {
        assert!(
            (1..=10).contains(&case.book_id),
            "case {} book_id should stay in the 10-book golden range",
            case.name
        );
        let source_path = root
            .join("fixtures/corpora/gutenberg_strip_golden")
            .join(&case.source_path);
        let source = std::fs::read(&source_path).expect("golden source reads");
        assert_eq!(
            hash_uri(&source),
            case.source_sha256,
            "case {} source hash pin",
            case.name
        );
        let observed = strip_gutenberg_d3(&source);

        match (
            case.expected_body,
            case.expected_post_strip_sha256,
            case.expected_drop_reason,
        ) {
            (Some(expected), Some(expected_post_strip_sha256), None) => {
                assert_eq!(
                    hash_uri(expected.as_bytes()),
                    expected_post_strip_sha256,
                    "case {} expected body hash pin",
                    case.name
                );
                let stripped = observed.unwrap_or_else(|reason| {
                    panic!("case {} unexpectedly dropped with {reason}", case.name)
                });
                assert_eq!(stripped.body, expected, "case {}", case.name);
                assert_eq!(
                    stripped.post_strip_sha256.to_string(),
                    expected_post_strip_sha256,
                    "case {} post-strip hash pin",
                    case.name
                );

                let rerun = strip_gutenberg_d3(stripped.body.as_bytes())
                    .expect_err("stripped body should not strip a second time");
                assert_eq!(rerun, GutenbergD3DropReason::GutenbergMarkerMissing);
            }
            (None, None, Some(expected_reason)) => {
                let reason = observed.expect_err("golden drop case should reject");
                assert_eq!(
                    reason.as_str(),
                    expected_reason.as_str(),
                    "case {}",
                    case.name
                );
            }
            _ => panic!(
                "case {} must specify exactly one expected body/hash pair or drop reason",
                case.name
            ),
        }
    }
}

#[test]
fn d3_normalization_strips_bom_normalizes_line_endings_and_nfc() {
    let source = b"\xef\xbb\xbfIntro\r\n*** START OF THIS PROJECT GUTENBERG EBOOK NORMALIZE ***\r\nCafe\xcc\x81\rLine two.\r\n*** END OF THIS PROJECT GUTENBERG EBOOK NORMALIZE ***\r\nTrailer";
    let normalized = normalize_gutenberg_d3_text(source).expect("source normalizes");
    assert!(!normalized.starts_with('\u{feff}'));
    assert!(!normalized.contains('\r'));
    assert!(normalized.contains("Café\nLine two."));

    let stripped = strip_gutenberg_d3(source).expect("normalized source strips");
    assert_eq!(stripped.body, "Café\nLine two.");
}

#[test]
fn d3_preserves_body_trailing_spaces() {
    let source = b"Intro\n*** START OF THE PROJECT GUTENBERG EBOOK BODY WHITESPACE ***\nTrailing spaces stay.   \nNext line.\n*** END OF THE PROJECT GUTENBERG EBOOK BODY WHITESPACE ***\nTrailer";
    let stripped = strip_gutenberg_d3(source).expect("source strips");
    assert_eq!(stripped.body, "Trailing spaces stay.   \nNext line.");
}

#[test]
fn d3_rejects_invalid_utf8_with_taxonomy_reason() {
    let reason = strip_gutenberg_d3(b"\xffnot valid utf8").expect_err("invalid UTF-8 rejects");
    assert_eq!(reason, GutenbergD3DropReason::InvalidUtf8);
    assert_eq!(reason.as_str(), "invalid_utf8");
}

#[test]
fn d3_taxonomy_pins_source_decode_and_empty_after_strip_reasons() {
    assert_eq!(
        GutenbergD3DropReason::SourceDecodeFailed.as_str(),
        "source_decode_failed"
    );
    assert_eq!(
        empty_after_strip_reason(0),
        Some(GutenbergD3DropReason::EmptyAfterStrip)
    );
    assert_eq!(empty_after_strip_reason(1), None);
}

#[test]
fn d3_marker_drop_cap_uses_strict_five_percent_rule() {
    assert!(!marker_missing_drop_cap_breached(5, 100));
    assert!(marker_missing_drop_cap_breached(6, 100));
    assert!(!marker_missing_drop_cap_breached(0, 10));
    assert!(marker_missing_drop_cap_breached(1, 10));
}

#[test]
fn d3_regex_constants_match_rfc_literals() {
    assert_eq!(
        GUTENBERG_D3_HEADER_REGEX_PATTERN,
        r"(?im-s)\A(?s:.*?)^[ \t]*\*{3}[ \t]*START OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*\n"
    );
    assert_eq!(
        GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN,
        r"(?im-s)\n[ \t]*\*{3}[ \t]*END OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*"
    );
}

#[derive(Debug, Deserialize)]
struct GoldenFixture {
    schema: String,
    case_count: usize,
    cases: Vec<GoldenCase>,
}

#[derive(Debug, Deserialize)]
struct GoldenCase {
    book_id: u32,
    name: String,
    source_path: String,
    source_sha256: String,
    expected_body: Option<String>,
    expected_post_strip_sha256: Option<String>,
    expected_drop_reason: Option<String>,
}

fn read_fixture(root: &Path) -> GoldenFixture {
    let manifest_path = root.join("fixtures/corpora/gutenberg_strip_golden/expected.toml");
    let text = std::fs::read_to_string(manifest_path).expect("golden manifest reads");
    toml::from_str(&text).expect("golden manifest parses")
}

fn hash_uri(bytes: &[u8]) -> String {
    gbf_foundation::sha256(bytes).to_string()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("gbf-data has a workspace parent")
        .to_path_buf()
}
