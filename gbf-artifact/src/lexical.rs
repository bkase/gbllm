//! F-S3 charset_v1 lexical ids and checked token sequence types.

use std::error::Error;
use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// Charset_v1 character id.
pub type CharId = u8;

/// Number of ids in charset_v1.
pub const VOCAB_SIZE: usize = 80;

/// Id reserved for forward charset expansion. It is invalid in v1 streams.
pub const RESERVED_ID: CharId = 76;

/// Beginning-of-sequence control token id.
pub const BOS_ID: CharId = 77;

/// End-of-sequence control token id.
pub const EOS_ID: CharId = 78;

/// Unknown-character control token id.
pub const UNK_ID: CharId = 79;

/// Returns true for ids in the printable charset row, including newline id 75.
#[must_use]
pub const fn is_printable_id(c: CharId) -> bool {
    c <= 75
}

/// Returns true for charset_v1 control token ids.
#[must_use]
pub const fn is_control_id(c: CharId) -> bool {
    c == BOS_ID || c == EOS_ID || c == UNK_ID
}

/// Returns true for normalized text stream ids.
#[must_use]
pub const fn is_text_char_id(c: CharId) -> bool {
    is_printable_id(c) || c == UNK_ID
}

/// Checked normalized corpus/prompt text sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct TextCharSeq(Vec<CharId>);

impl TextCharSeq {
    /// Build a normalized text sequence.
    ///
    /// Rejects reserved id 76, `<bos>` id 77, `<eos>` id 78, and ids outside
    /// charset_v1.
    pub fn new(ids: Vec<CharId>) -> Result<Self, LexicalError> {
        match validate_text_char_ids(&ids) {
            Ok(()) => Ok(Self(ids)),
            Err(error) => {
                log_rejection(&error);
                Err(error)
            }
        }
    }

    /// Borrow the checked ids.
    #[must_use]
    pub fn as_slice(&self) -> &[CharId] {
        &self.0
    }

    /// Consume the wrapper and return the checked ids.
    #[must_use]
    pub fn into_vec(self) -> Vec<CharId> {
        self.0
    }

    /// Number of checked ids.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Whether the sequence is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl<'de> Deserialize<'de> for TextCharSeq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ids = Vec::<CharId>::deserialize(deserializer)?;
        Self::new(ids).map_err(serde::de::Error::custom)
    }
}

/// Checked model-side token sequence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ModelTokenSeq(Vec<CharId>);

impl ModelTokenSeq {
    /// Build a model-side token sequence.
    ///
    /// Accepts printable ids and control token ids, but rejects reserved id 76
    /// and ids outside charset_v1.
    pub fn new(ids: Vec<CharId>) -> Result<Self, LexicalError> {
        match validate_model_token_ids(&ids) {
            Ok(()) => Ok(Self(ids)),
            Err(error) => {
                log_rejection(&error);
                Err(error)
            }
        }
    }

    /// Borrow the checked ids.
    #[must_use]
    pub fn as_slice(&self) -> &[CharId] {
        &self.0
    }

    /// Consume the wrapper and return the checked ids.
    #[must_use]
    pub fn into_vec(self) -> Vec<CharId> {
        self.0
    }
}

impl<'de> Deserialize<'de> for ModelTokenSeq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let ids = Vec::<CharId>::deserialize(deserializer)?;
        Self::new(ids).map_err(serde::de::Error::custom)
    }
}

/// Lexical constructor failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LexicalError {
    /// Reserved id 76 appeared in a v1 stream.
    ReservedId76 {
        /// Position of the offending id.
        position: usize,
    },
    /// `<bos>` or `<eos>` appeared in a normalized text stream.
    ControlIdInTextStream {
        /// Offending control id.
        id: CharId,
        /// Position of the offending id.
        position: usize,
    },
    /// Id outside the charset_v1 range appeared.
    OutOfRange {
        /// Offending id.
        id: CharId,
        /// Position of the offending id.
        position: usize,
    },
}

impl LexicalError {
    /// Stable rejection kind for logs/tests.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::ReservedId76 { .. } => "ReservedId76",
            Self::ControlIdInTextStream { .. } => "ControlIdInTextStream",
            Self::OutOfRange { .. } => "OutOfRange",
        }
    }

    /// Offending id.
    #[must_use]
    pub const fn id(&self) -> CharId {
        match self {
            Self::ReservedId76 { .. } => RESERVED_ID,
            Self::ControlIdInTextStream { id, .. } | Self::OutOfRange { id, .. } => *id,
        }
    }

    /// Offending position.
    #[must_use]
    pub const fn position(&self) -> usize {
        match self {
            Self::ReservedId76 { position }
            | Self::ControlIdInTextStream { position, .. }
            | Self::OutOfRange { position, .. } => *position,
        }
    }
}

impl fmt::Display for LexicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedId76 { position } => {
                write!(f, "reserved charset id 76 at position {position}")
            }
            Self::ControlIdInTextStream { id, position } => {
                write!(
                    f,
                    "control charset id {id} in text stream at position {position}"
                )
            }
            Self::OutOfRange { id, position } => {
                write!(f, "charset id {id} out of range at position {position}")
            }
        }
    }
}

impl Error for LexicalError {}

fn validate_text_char_ids(ids: &[CharId]) -> Result<(), LexicalError> {
    for (position, &id) in ids.iter().enumerate() {
        if id > UNK_ID {
            return Err(LexicalError::OutOfRange { id, position });
        }
        if id == RESERVED_ID {
            return Err(LexicalError::ReservedId76 { position });
        }
        if id == BOS_ID || id == EOS_ID {
            return Err(LexicalError::ControlIdInTextStream { id, position });
        }
    }
    Ok(())
}

fn validate_model_token_ids(ids: &[CharId]) -> Result<(), LexicalError> {
    for (position, &id) in ids.iter().enumerate() {
        if id > UNK_ID {
            return Err(LexicalError::OutOfRange { id, position });
        }
        if id == RESERVED_ID {
            return Err(LexicalError::ReservedId76 { position });
        }
    }
    Ok(())
}

fn log_rejection(error: &LexicalError) {
    tracing::warn!(
        target: "gbf_artifact::lexical",
        event_name = "s3::lexical::reject_construction",
        error_kind = error.kind(),
        id = error.id(),
        position = error.position(),
        callsite_module = module_path!(),
    );
}
