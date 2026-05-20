//! Project Gutenberg corpus helpers for F-S4.

use std::sync::OnceLock;

use gbf_foundation::{Hash256, sha256};
use regex::Regex;
use unicode_normalization::UnicodeNormalization;

/// F-S4 D3 header recognizer, pinned verbatim.
pub const GUTENBERG_D3_HEADER_REGEX_PATTERN: &str = r"(?im-s)\A(?s:.*?)^[ \t]*\*{3}[ \t]*START OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*\n";

/// F-S4 D3 footer-marker recognizer, pinned verbatim.
pub const GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN: &str =
    r"(?im-s)\n[ \t]*\*{3}[ \t]*END OF (?:THIS |THE )?PROJECT GUTENBERG EBOOK\b(?s:.*?\*{3})[ \t]*";

/// Marker-missing hard cap from F-S4 D3.
pub const GUTENBERG_D3_MARKER_MISSING_DROP_MAX_FRACTION: f64 = 0.05;

/// Reasons owned by the D3 stripping boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GutenbergD3DropReason {
    SourceDecodeFailed,
    InvalidUtf8,
    GutenbergMarkerMissing,
    EmptyAfterStrip,
}

impl GutenbergD3DropReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SourceDecodeFailed => "source_decode_failed",
            Self::InvalidUtf8 => "invalid_utf8",
            Self::GutenbergMarkerMissing => "gutenberg_marker_missing",
            Self::EmptyAfterStrip => "empty_after_strip",
        }
    }
}

impl std::fmt::Display for GutenbergD3DropReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Normalized and stripped text plus provenance hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GutenbergStrippedText {
    pub normalized_text: String,
    pub body: String,
    pub normalized_text_sha256: Hash256,
    pub post_strip_sha256: Hash256,
    pub header_end: usize,
    pub footer_start: usize,
}

/// Apply F-S4 D3 UTF-8 validation, BOM removal, CR/LF normalization, NFC,
/// and header/footer marker stripping.
pub fn strip_gutenberg_d3(
    decoded_utf8_bytes: &[u8],
) -> Result<GutenbergStrippedText, GutenbergD3DropReason> {
    let normalized = normalize_gutenberg_d3_text(decoded_utf8_bytes)?;
    strip_gutenberg_d3_normalized_text(&normalized)
}

/// Apply the F-S4 D3 text normalization before marker recognition.
pub fn normalize_gutenberg_d3_text(
    decoded_utf8_bytes: &[u8],
) -> Result<String, GutenbergD3DropReason> {
    let text =
        std::str::from_utf8(decoded_utf8_bytes).map_err(|_| GutenbergD3DropReason::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let line_normalized = normalize_crlf_cr_to_lf(text);
    Ok(line_normalized.nfc().collect())
}

/// Strip markers from already-normalized D3 text.
pub fn strip_gutenberg_d3_normalized_text(
    normalized_text: &str,
) -> Result<GutenbergStrippedText, GutenbergD3DropReason> {
    let header = header_regex()
        .find(normalized_text)
        .ok_or(GutenbergD3DropReason::GutenbergMarkerMissing)?;
    let footer = footer_regex()
        .find_iter(normalized_text)
        .filter(|candidate| candidate.start() >= header.end())
        .max_by_key(|candidate| candidate.start())
        .ok_or(GutenbergD3DropReason::GutenbergMarkerMissing)?;

    if header.end() > footer.start() {
        return Err(GutenbergD3DropReason::GutenbergMarkerMissing);
    }

    let body = normalized_text[header.end()..footer.start()].to_owned();
    Ok(GutenbergStrippedText {
        normalized_text: normalized_text.to_owned(),
        normalized_text_sha256: sha256(normalized_text.as_bytes()),
        post_strip_sha256: sha256(body.as_bytes()),
        body,
        header_end: header.end(),
        footer_start: footer.start(),
    })
}

/// Convert CRLF and bare CR to LF before marker recognition.
#[must_use]
pub fn normalize_crlf_cr_to_lf(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            if chars.peek() == Some(&'\n') {
                chars.next();
            }
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}

/// Return the D3 empty-body drop reason after charset normalization has
/// produced zero body token ids.
#[must_use]
pub const fn empty_after_strip_reason(
    post_charset_body_token_count: usize,
) -> Option<GutenbergD3DropReason> {
    if post_charset_body_token_count == 0 {
        Some(GutenbergD3DropReason::EmptyAfterStrip)
    } else {
        None
    }
}

/// True when marker-missing drops breach the D3 5% cap.
#[must_use]
pub fn marker_missing_drop_cap_breached(marker_missing_drops: u32, book_count: u32) -> bool {
    u128::from(marker_missing_drops) * 20 > u128::from(book_count)
}

fn header_regex() -> &'static Regex {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    HEADER.get_or_init(|| {
        Regex::new(GUTENBERG_D3_HEADER_REGEX_PATTERN).expect("D3 header regex compiles")
    })
}

fn footer_regex() -> &'static Regex {
    static FOOTER: OnceLock<Regex> = OnceLock::new();
    FOOTER.get_or_init(|| {
        Regex::new(GUTENBERG_D3_FOOTER_MARKER_REGEX_PATTERN)
            .expect("D3 footer marker regex compiles")
    })
}
