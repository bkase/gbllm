//! Byte-level BPE tokenizer: Rust port of `training/gbtrain/tokenizer.py`.
//!
//! This is the host-side subword deploy surface for the MoE model (build-order
//! Step 5). It loads the `gbllm_bpe.v2` artifact (vocab + ordered merges +
//! explicit id->bytes) and reproduces the Python greedy BPE **byte-for-byte**.
//!
//! Parity contract (see `training/gbtrain/tokenizer.py` and
//! `training/gbtrain/conformance.py`):
//!
//! * **Pre-tokenizer.** A total, look-around-free regex splits text into
//!   chunks; `chunks.join("") == text` for any input. The Python `re` and
//!   Rust `regex` crates both compile the pattern with Unicode `\w`/`\d`/`\s`
//!   semantics enabled by default; the frozen conformance vectors are the
//!   authoritative parity gate, not the pattern string.
//! * **Encode.** Each chunk's UTF-8 bytes are the initial id sequence
//!   (`0..=255`). The canonical merge loop *repeatedly finds the pair with the
//!   minimum merge rank, merges the leftmost occurrence, and repeats until no
//!   adjacent pair has a rank.* Merge `i` produces token id `256 + i`.
//! * **Decode.** Concatenate each id's literal bytes (`id_bytes`) and UTF-8
//!   decode with lossy replacement — lossless on encoder-produced streams.
//!
//! The vocabulary is 1024 tokens, so ids fit `u16`. The core encode/decode
//! path operates on plain slices and could be lifted into a `no_std` builder
//! later; JSON loading (`serde_json`) is std-only host convenience.

use std::collections::HashMap;

use serde::Deserialize;

/// Base vocabulary size: the 256 single-byte tokens (ids `0..=255`).
pub const BASE_VOCAB: usize = 256;

/// Token id width. The deployed vocab is 1024, so ids fit `u16`.
pub type TokenId = u16;

/// Errors from loading or validating a BPE artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BpeError {
    /// The artifact JSON could not be parsed.
    Parse(String),
    /// A merge references an id that is not yet defined at its step.
    UndefinedMergeId {
        /// The merge step (0-based).
        step: usize,
        /// The offending `(a, b)` pair.
        pair: (TokenId, TokenId),
    },
    /// The artifact's declared `vocab_size` disagrees with the replayed size.
    VocabSizeMismatch {
        /// The size declared in the artifact header.
        declared: usize,
        /// The size implied by `256 + merges.len()`.
        replayed: usize,
    },
    /// The `base_vocab` header is not 256.
    BaseVocabMismatch {
        /// The value found in the artifact.
        found: usize,
    },
    /// The `id_bytes_hex` table is missing an entry, or has the wrong count.
    IdBytesTable(String),
    /// The replayed id->bytes disagree with the artifact's `id_bytes_hex`.
    IdBytesMismatch {
        /// The token id whose bytes disagreed.
        id: TokenId,
    },
}

impl std::fmt::Display for BpeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BpeError::Parse(msg) => write!(f, "failed to parse BPE artifact: {msg}"),
            BpeError::UndefinedMergeId { step, pair } => {
                write!(f, "merge {step} references undefined id: {pair:?}")
            }
            BpeError::VocabSizeMismatch { declared, replayed } => {
                write!(f, "vocab_size mismatch: {declared} != {replayed}")
            }
            BpeError::BaseVocabMismatch { found } => {
                write!(f, "base_vocab mismatch: {found} != {BASE_VOCAB}")
            }
            BpeError::IdBytesTable(msg) => write!(f, "id_bytes_hex table error: {msg}"),
            BpeError::IdBytesMismatch { id } => {
                write!(f, "vocab mismatch at id {id}: artifact bytes != replay")
            }
        }
    }
}

impl std::error::Error for BpeError {}

/// Raw shape of the `gbllm_bpe.v2` artifact JSON.
#[derive(Debug, Deserialize)]
struct BpeArtifact {
    base_vocab: usize,
    vocab_size: usize,
    merges: Vec<(TokenId, TokenId)>,
    id_bytes_hex: HashMap<String, String>,
}

/// A loaded byte-level BPE model: greedy encode + literal-byte decode.
#[derive(Debug, Clone)]
pub struct BpeModel {
    /// Ordered merge pairs; merge `i` creates token id `256 + i`.
    merges: Vec<(TokenId, TokenId)>,
    /// Merge-rank lookup: `(a, b) -> i`. Lower rank merges first.
    rank: HashMap<(TokenId, TokenId), usize>,
    /// `id -> literal bytes` for every token id in the vocabulary.
    id_bytes: Vec<Vec<u8>>,
}

impl BpeModel {
    /// Load and validate a `gbllm_bpe.v2` artifact from JSON.
    ///
    /// Mirrors `BPEModel.from_json`: replays the merges into an id->bytes
    /// table and checks it byte-for-byte against the artifact's
    /// `id_bytes_hex`, so a corrupt or mismatched artifact fails loudly.
    pub fn from_json(text: &str) -> Result<Self, BpeError> {
        let artifact: BpeArtifact =
            serde_json::from_str(text).map_err(|e| BpeError::Parse(e.to_string()))?;

        if artifact.base_vocab != BASE_VOCAB {
            return Err(BpeError::BaseVocabMismatch {
                found: artifact.base_vocab,
            });
        }

        let replayed_size = BASE_VOCAB + artifact.merges.len();
        if artifact.vocab_size != replayed_size {
            return Err(BpeError::VocabSizeMismatch {
                declared: artifact.vocab_size,
                replayed: replayed_size,
            });
        }

        // Replay the merges into the id->bytes table, mirroring `__post_init__`.
        let mut id_bytes: Vec<Vec<u8>> = (0..BASE_VOCAB).map(|i| vec![i as u8]).collect();
        for (i, &(a, b)) in artifact.merges.iter().enumerate() {
            let defined = (BASE_VOCAB + i) as TokenId;
            if a >= defined || b >= defined {
                return Err(BpeError::UndefinedMergeId {
                    step: i,
                    pair: (a, b),
                });
            }
            let mut bytes = id_bytes[a as usize].clone();
            bytes.extend_from_slice(&id_bytes[b as usize]);
            id_bytes.push(bytes);
        }

        // Strict integrity: a COMPLETE id->bytes map that matches the replay.
        if artifact.id_bytes_hex.len() != replayed_size {
            return Err(BpeError::IdBytesTable(format!(
                "id_bytes_hex has {} entries, expected {replayed_size}",
                artifact.id_bytes_hex.len()
            )));
        }
        for (id, expect) in id_bytes.iter().enumerate() {
            let hex = artifact
                .id_bytes_hex
                .get(&id.to_string())
                .ok_or_else(|| BpeError::IdBytesTable(format!("missing entry for id {id}")))?;
            let actual = decode_hex(hex)
                .ok_or_else(|| BpeError::IdBytesTable(format!("invalid hex for id {id}: {hex}")))?;
            if &actual != expect {
                return Err(BpeError::IdBytesMismatch { id: id as TokenId });
            }
        }

        let rank = artifact
            .merges
            .iter()
            .enumerate()
            .map(|(i, &pair)| (pair, i))
            .collect();

        Ok(BpeModel {
            merges: artifact.merges,
            rank,
            id_bytes,
        })
    }

    /// Total token count: `256 + merges.len()`.
    pub fn vocab_size(&self) -> usize {
        BASE_VOCAB + self.merges.len()
    }

    /// Ordered merge program used by the canonical encoder.
    ///
    /// Merge `i` consumes the returned pair at index `i` and produces token
    /// id `256 + i`. Cartridge builders use this read-only view to embed the
    /// exact tokenizer program beside an interactive on-device prompt shell;
    /// keeping the source table here prevents the ROM tokenizer from being
    /// reconstructed heuristically from decoded token bytes.
    pub fn merges(&self) -> &[(TokenId, TokenId)] {
        &self.merges
    }

    /// Longest token's byte length (the on-device render-buffer bound).
    pub fn max_token_len(&self) -> usize {
        self.id_bytes.iter().map(Vec::len).max().unwrap_or(1)
    }

    /// The literal bytes for a token id, or `None` if out of range.
    pub fn id_bytes(&self, id: TokenId) -> Option<&[u8]> {
        self.id_bytes.get(id as usize).map(Vec::as_slice)
    }

    /// Encode text into token ids via greedy byte-level BPE.
    ///
    /// Byte-for-byte equivalent to `BPEModel.encode`: pre-tokenize into
    /// chunks, then merge each chunk's UTF-8 bytes with the canonical
    /// lowest-rank-first, leftmost-occurrence loop.
    pub fn encode(&self, text: &str) -> Vec<TokenId> {
        let mut out = Vec::new();
        for chunk in pretokenize(text) {
            let mut ids: Vec<TokenId> = chunk.bytes().map(TokenId::from).collect();
            self.merge_chunk(&mut ids);
            out.extend_from_slice(&ids);
        }
        out
    }

    /// Canonical BPE merge of one chunk's id sequence, in place.
    ///
    /// Repeatedly finds the adjacent pair with the minimum merge rank and
    /// merges its leftmost occurrence, until no adjacent pair has a rank.
    fn merge_chunk(&self, ids: &mut Vec<TokenId>) {
        if ids.len() < 2 {
            return;
        }
        loop {
            let mut best_rank: Option<usize> = None;
            let mut best_pos: usize = 0;
            for pos in 0..ids.len() - 1 {
                if let Some(&rank) = self.rank.get(&(ids[pos], ids[pos + 1]))
                    && best_rank.is_none_or(|best| rank < best)
                {
                    best_rank = Some(rank);
                    best_pos = pos;
                }
            }
            let Some(rank) = best_rank else {
                return;
            };
            let new_id = (BASE_VOCAB + rank) as TokenId;
            ids[best_pos] = new_id;
            ids.remove(best_pos + 1);
        }
    }

    /// Decode ids to their concatenated literal bytes.
    ///
    /// Ids out of range contribute nothing (they cannot appear in an
    /// encoder-produced stream). Pair with `String::from_utf8_lossy` for text.
    pub fn decode_bytes(&self, ids: &[TokenId]) -> Vec<u8> {
        let mut buf = Vec::new();
        for &id in ids {
            if let Some(bytes) = self.id_bytes(id) {
                buf.extend_from_slice(bytes);
            }
        }
        buf
    }

    /// Decode ids to a `String`, mirroring `BPEModel.decode`
    /// (`utf-8` with lossy replacement — lossless on complete streams).
    pub fn decode(&self, ids: &[TokenId]) -> String {
        String::from_utf8_lossy(&self.decode_bytes(ids)).into_owned()
    }
}

/// Decode a lowercase/uppercase hex string into bytes, or `None` if malformed.
fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
    }
    Some(out)
}

/// Split text into pre-token chunks. Total: `chunks.join("") == text`.
///
/// Rust port of the look-around-free Python pattern
/// `(?s) ?[^\W\d_]+| ?\d+| ?[^\s\w]+| ?_+|\s+|.`. Both engines enable Unicode
/// `\w`/`\d`/`\s` by default, so the classes agree; the frozen conformance
/// vectors are the authoritative parity gate.
pub fn pretokenize(text: &str) -> Vec<&str> {
    PRETOKENIZER.find_iter(text).map(|m| m.as_str()).collect()
}

use std::sync::LazyLock;

use regex::Regex;

/// The pre-tokenizer, compiled once. Matches the Python `_PATTERN` exactly.
static PRETOKENIZER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s) ?[^\W\d_]+| ?\d+| ?[^\s\w]+| ?_+|\s+|.")
        .expect("BPE pre-tokenizer pattern is a valid regex")
});
