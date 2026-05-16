//! Raw UTF-8 to `charset_v1` token normalization.

use std::error::Error;
use std::fmt;

use gbf_artifact::TextCharSeq;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::unmappable::{decide_drop, encode_charset_v1, unk_fraction};
use super::whitespace::normalize_whitespace;

/// Normalized text sequence and per-example unmappable accounting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextCharSeqWithStats {
    pub tokens: TextCharSeq,
    pub unk_count_in_example: u32,
    pub dropped: bool,
    pub drop_reason: Option<DropReason>,
}

impl TextCharSeqWithStats {
    #[must_use]
    pub fn post_token_count(&self) -> usize {
        self.tokens.len()
    }
}

/// Reason an example was dropped by the charset gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    UnmappableFractionExceeded,
}

/// Normalize one raw UTF-8 example into `charset_v1` ids.
pub fn normalize_raw(raw_example_bytes: &[u8]) -> Result<TextCharSeqWithStats, CharsetError> {
    let raw = std::str::from_utf8(raw_example_bytes).map_err(CharsetError::InvalidUtf8)?;
    let nfc = unicode_nfc(raw);
    let stripped = strip_combining_accents(&nfc);
    let preserved = preserve_case(&stripped);
    let folded = fold_quotes_dashes_ellipsis(&preserved);
    let whitespace_normalized = normalize_whitespace(&folded);
    let (ids, unk_count_in_example) = encode_charset_v1(&whitespace_normalized);
    let tokens = TextCharSeq::new(ids).map_err(CharsetError::Lexical)?;

    let mut stats = TextCharSeqWithStats {
        tokens,
        unk_count_in_example,
        dropped: false,
        drop_reason: None,
    };
    if decide_drop(unk_fraction(&stats)) {
        stats.dropped = true;
        stats.drop_reason = Some(DropReason::UnmappableFractionExceeded);
    }
    Ok(stats)
}

/// Step 1: canonical Unicode NFC.
#[must_use]
pub fn unicode_nfc(input: &str) -> String {
    input.nfc().collect()
}

/// Step 2: remove combining accent marks after decomposition.
#[must_use]
pub fn strip_combining_accents(input: &str) -> String {
    input
        .nfd()
        .filter(|ch| !is_combining_mark(*ch))
        .collect::<String>()
        .nfc()
        .collect()
}

/// Step 3: preserve case. This no-op function exists to pin the RFC order.
#[must_use]
pub fn preserve_case(input: &str) -> String {
    input.to_owned()
}

/// Step 4: fold quotation marks, dashes, and ellipsis to ASCII.
#[must_use]
pub fn fold_quotes_dashes_ellipsis(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' | '\u{201A}' => out.push('\''),
            '\u{00AB}' | '\u{00BB}' | '\u{201C}' | '\u{201D}' => out.push('"'),
            '\u{2013}' => out.push('-'),
            '\u{2014}' => out.push_str("--"),
            '\u{2026}' => out.push_str("..."),
            _ => out.push(ch),
        }
    }
    out
}

/// Charset normalization failure.
#[derive(Debug)]
pub enum CharsetError {
    InvalidUtf8(std::str::Utf8Error),
    Lexical(gbf_artifact::LexicalError),
    ReservedIdInInput {
        position: usize,
    },
    PostShaMismatch {
        expected: gbf_foundation::Hash256,
        observed: gbf_foundation::Hash256,
    },
    SpecMismatch,
    CanonicalJson(gbf_foundation::CanonicalJsonError),
    SerdeJson(serde_json::Error),
    ExpectedObjectForSelfHash,
}

impl fmt::Display for CharsetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUtf8(error) => write!(f, "raw example is not valid UTF-8: {error}"),
            Self::Lexical(error) => write!(f, "{error}"),
            Self::ReservedIdInInput { position } => {
                write!(f, "reserved charset id 76 at position {position}")
            }
            Self::PostShaMismatch { expected, observed } => {
                write!(
                    f,
                    "post-normalized sha mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::SpecMismatch => f.write_str("charset_v1 requires the pinned LexicalSpec_v1"),
            Self::CanonicalJson(error) => write!(f, "{error}"),
            Self::SerdeJson(error) => write!(f, "{error}"),
            Self::ExpectedObjectForSelfHash => {
                f.write_str("charset self-hash requires a top-level object")
            }
        }
    }
}

impl Error for CharsetError {}
