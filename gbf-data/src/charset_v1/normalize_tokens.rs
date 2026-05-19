//! Idempotence operator over already-normalized `charset_v1` ids.

use gbf_artifact::{RESERVED_ID, TextCharSeq};

use super::normalize_raw::CharsetError;

/// Canonicalize an already-normalized text sequence.
#[must_use]
pub fn normalize_tokens(text_char_seq: TextCharSeq) -> TextCharSeq {
    TextCharSeq::new(text_char_seq.into_vec()).expect("TextCharSeq input is already checked")
}

/// Loader-facing checked token-id normalization.
pub fn normalize_token_ids(ids: Vec<u8>) -> Result<TextCharSeq, CharsetError> {
    if let Some(position) = ids.iter().position(|id| *id == RESERVED_ID) {
        return Err(CharsetError::ReservedIdInInput { position });
    }
    TextCharSeq::new(ids).map_err(CharsetError::Lexical)
}
