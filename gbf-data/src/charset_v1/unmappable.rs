//! `charset_v1` unmappable handling.

use gbf_artifact::UNK_ID;

use super::normalize_raw::TextCharSeqWithStats;

/// Drop examples whose unknown-token fraction is strictly greater than 2%.
pub const UNMAPPABLE_EXAMPLE_DROP_THRESHOLD: f64 = 0.02;

/// Encode post-whitespace normalized text to `charset_v1` ids.
#[must_use]
pub fn encode_charset_v1(input: &str) -> (Vec<u8>, u32) {
    let mut unk_count = 0_u32;
    let ids = input
        .chars()
        .map(|ch| match char_id(ch) {
            Some(id) => id,
            None => {
                unk_count += 1;
                UNK_ID
            }
        })
        .collect();
    (ids, unk_count)
}

/// Per-example unknown fraction after post-normalization tokenization.
#[must_use]
pub fn unk_fraction(example: &TextCharSeqWithStats) -> f64 {
    let token_count = example.tokens.len();
    if token_count == 0 {
        0.0
    } else {
        f64::from(example.unk_count_in_example) / token_count as f64
    }
}

/// Return true when an example must be dropped.
#[must_use]
pub fn decide_drop(unk_fraction: f64) -> bool {
    unk_fraction > UNMAPPABLE_EXAMPLE_DROP_THRESHOLD
}

fn char_id(ch: char) -> Option<u8> {
    match ch {
        'A'..='Z' => Some(ch as u8 - b'A'),
        'a'..='z' => Some(26 + (ch as u8 - b'a')),
        '0'..='9' => Some(52 + (ch as u8 - b'0')),
        ' ' => Some(62),
        '.' => Some(63),
        ',' => Some(64),
        '!' => Some(65),
        '?' => Some(66),
        '-' => Some(67),
        '\'' => Some(68),
        ':' => Some(69),
        ';' => Some(70),
        '(' => Some(71),
        ')' => Some(72),
        '"' => Some(73),
        '/' => Some(74),
        '\n' => Some(75),
        _ => None,
    }
}
