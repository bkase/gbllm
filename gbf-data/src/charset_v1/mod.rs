//! S3 `charset_v1` normalization pipeline.

pub mod normalize_raw;
pub mod normalize_tokens;
pub mod pipeline;
pub mod unmappable;
pub mod whitespace;

pub use normalize_raw::{
    CharsetError, DropReason, TextCharSeqWithStats, fold_quotes_dashes_ellipsis, normalize_raw,
    preserve_case, strip_combining_accents, unicode_nfc,
};
pub use normalize_tokens::{normalize_token_ids, normalize_tokens};
pub use pipeline::{
    CharsetInputs, CharsetProduct, CharsetSplit, DropEvent, s3_charset_v1, verify_charseq_sha256,
};
pub use unmappable::{
    UNMAPPABLE_EXAMPLE_DROP_THRESHOLD, decide_drop, encode_charset_v1, unk_fraction,
};
pub use whitespace::{
    collapse_internal_ascii_spaces, normalize_line_endings, normalize_whitespace, tabs_to_spaces,
    trim_trailing_ascii_spaces,
};
