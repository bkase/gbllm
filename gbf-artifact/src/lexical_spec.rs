//! F-S3 charset_v1 lexical and normalization specs.

use gbf_foundation::{CanonicalJson, DomainHash, Hash256, self_hash_omitting_fields};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::lexical::{BOS_ID, CharId, EOS_ID, RESERVED_ID, UNK_ID, VOCAB_SIZE};

const CRATE_NAME: &str = "gbf-artifact";
const SCHEMA_VERSION: &str = "1";
const LEXICAL_SPEC_SCHEMA_ID: &str = "lexical_spec.v1";
const NORMALIZATION_SPEC_SCHEMA_ID: &str = "normalization_spec.v1";

/// Domain separator for `lexical_spec.v1` self-hashes.
pub const LEXICAL_SPEC_HASH_DOMAIN_SEPARATOR: &[u8] =
    b"gbf:gbf-artifact:LexicalSpec_v1:lexical_spec.v1:1\0";

/// Domain separator for `normalization_spec.v1` self-hashes.
pub const NORMALIZATION_SPEC_HASH_DOMAIN_SEPARATOR: &[u8] =
    b"gbf:gbf-artifact:NormalizationSpec_v1:normalization_spec.v1:1\0";

/// Pinned charset_v1 normalization order.
pub const NORMALIZATION_ORDER_V1: [&str; 6] = [
    "nfc",
    "strip_combining_accents",
    "preserve_case",
    "fold_quotes_and_dashes",
    "whitespace",
    "unmappable",
];

/// Pinned charset_v1 table.
pub const CHARSET_V1: [Char; VOCAB_SIZE] = [
    Char::Printable {
        id: 0,
        codepoint: 'A',
    },
    Char::Printable {
        id: 1,
        codepoint: 'B',
    },
    Char::Printable {
        id: 2,
        codepoint: 'C',
    },
    Char::Printable {
        id: 3,
        codepoint: 'D',
    },
    Char::Printable {
        id: 4,
        codepoint: 'E',
    },
    Char::Printable {
        id: 5,
        codepoint: 'F',
    },
    Char::Printable {
        id: 6,
        codepoint: 'G',
    },
    Char::Printable {
        id: 7,
        codepoint: 'H',
    },
    Char::Printable {
        id: 8,
        codepoint: 'I',
    },
    Char::Printable {
        id: 9,
        codepoint: 'J',
    },
    Char::Printable {
        id: 10,
        codepoint: 'K',
    },
    Char::Printable {
        id: 11,
        codepoint: 'L',
    },
    Char::Printable {
        id: 12,
        codepoint: 'M',
    },
    Char::Printable {
        id: 13,
        codepoint: 'N',
    },
    Char::Printable {
        id: 14,
        codepoint: 'O',
    },
    Char::Printable {
        id: 15,
        codepoint: 'P',
    },
    Char::Printable {
        id: 16,
        codepoint: 'Q',
    },
    Char::Printable {
        id: 17,
        codepoint: 'R',
    },
    Char::Printable {
        id: 18,
        codepoint: 'S',
    },
    Char::Printable {
        id: 19,
        codepoint: 'T',
    },
    Char::Printable {
        id: 20,
        codepoint: 'U',
    },
    Char::Printable {
        id: 21,
        codepoint: 'V',
    },
    Char::Printable {
        id: 22,
        codepoint: 'W',
    },
    Char::Printable {
        id: 23,
        codepoint: 'X',
    },
    Char::Printable {
        id: 24,
        codepoint: 'Y',
    },
    Char::Printable {
        id: 25,
        codepoint: 'Z',
    },
    Char::Printable {
        id: 26,
        codepoint: 'a',
    },
    Char::Printable {
        id: 27,
        codepoint: 'b',
    },
    Char::Printable {
        id: 28,
        codepoint: 'c',
    },
    Char::Printable {
        id: 29,
        codepoint: 'd',
    },
    Char::Printable {
        id: 30,
        codepoint: 'e',
    },
    Char::Printable {
        id: 31,
        codepoint: 'f',
    },
    Char::Printable {
        id: 32,
        codepoint: 'g',
    },
    Char::Printable {
        id: 33,
        codepoint: 'h',
    },
    Char::Printable {
        id: 34,
        codepoint: 'i',
    },
    Char::Printable {
        id: 35,
        codepoint: 'j',
    },
    Char::Printable {
        id: 36,
        codepoint: 'k',
    },
    Char::Printable {
        id: 37,
        codepoint: 'l',
    },
    Char::Printable {
        id: 38,
        codepoint: 'm',
    },
    Char::Printable {
        id: 39,
        codepoint: 'n',
    },
    Char::Printable {
        id: 40,
        codepoint: 'o',
    },
    Char::Printable {
        id: 41,
        codepoint: 'p',
    },
    Char::Printable {
        id: 42,
        codepoint: 'q',
    },
    Char::Printable {
        id: 43,
        codepoint: 'r',
    },
    Char::Printable {
        id: 44,
        codepoint: 's',
    },
    Char::Printable {
        id: 45,
        codepoint: 't',
    },
    Char::Printable {
        id: 46,
        codepoint: 'u',
    },
    Char::Printable {
        id: 47,
        codepoint: 'v',
    },
    Char::Printable {
        id: 48,
        codepoint: 'w',
    },
    Char::Printable {
        id: 49,
        codepoint: 'x',
    },
    Char::Printable {
        id: 50,
        codepoint: 'y',
    },
    Char::Printable {
        id: 51,
        codepoint: 'z',
    },
    Char::Printable {
        id: 52,
        codepoint: '0',
    },
    Char::Printable {
        id: 53,
        codepoint: '1',
    },
    Char::Printable {
        id: 54,
        codepoint: '2',
    },
    Char::Printable {
        id: 55,
        codepoint: '3',
    },
    Char::Printable {
        id: 56,
        codepoint: '4',
    },
    Char::Printable {
        id: 57,
        codepoint: '5',
    },
    Char::Printable {
        id: 58,
        codepoint: '6',
    },
    Char::Printable {
        id: 59,
        codepoint: '7',
    },
    Char::Printable {
        id: 60,
        codepoint: '8',
    },
    Char::Printable {
        id: 61,
        codepoint: '9',
    },
    Char::Printable {
        id: 62,
        codepoint: ' ',
    },
    Char::Printable {
        id: 63,
        codepoint: '.',
    },
    Char::Printable {
        id: 64,
        codepoint: ',',
    },
    Char::Printable {
        id: 65,
        codepoint: '!',
    },
    Char::Printable {
        id: 66,
        codepoint: '?',
    },
    Char::Printable {
        id: 67,
        codepoint: '-',
    },
    Char::Printable {
        id: 68,
        codepoint: '\'',
    },
    Char::Printable {
        id: 69,
        codepoint: ':',
    },
    Char::Printable {
        id: 70,
        codepoint: ';',
    },
    Char::Printable {
        id: 71,
        codepoint: '(',
    },
    Char::Printable {
        id: 72,
        codepoint: ')',
    },
    Char::Printable {
        id: 73,
        codepoint: '"',
    },
    Char::Printable {
        id: 74,
        codepoint: '/',
    },
    Char::Printable {
        id: 75,
        codepoint: '\n',
    },
    Char::Reserved { id: RESERVED_ID },
    Char::Control {
        id: BOS_ID,
        token: ControlTokenName::Bos,
    },
    Char::Control {
        id: EOS_ID,
        token: ControlTokenName::Eos,
    },
    Char::Control {
        id: UNK_ID,
        token: ControlTokenName::Unk,
    },
];

/// One charset_v1 table entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Char {
    /// Printable ASCII codepoint entry.
    Printable {
        /// Character id.
        id: CharId,
        /// ASCII codepoint.
        codepoint: char,
    },
    /// Reserved id 76.
    Reserved {
        /// Character id.
        id: CharId,
    },
    /// Control token entry.
    Control {
        /// Character id.
        id: CharId,
        /// Stable control token name.
        token: ControlTokenName,
    },
}

impl Char {
    /// Return the table id.
    #[must_use]
    pub const fn id(self) -> CharId {
        match self {
            Self::Printable { id, .. } | Self::Reserved { id } | Self::Control { id, .. } => id,
        }
    }
}

/// Stable control-token names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlTokenName {
    /// `<bos>`.
    Bos,
    /// `<eos>`.
    Eos,
    /// `<unk>`.
    Unk,
}

/// Control token ids recorded in `lexical_spec.v1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlTokens {
    /// Beginning-of-sequence token id.
    pub bos: CharId,
    /// End-of-sequence token id.
    pub eos: CharId,
    /// Unknown-character token id.
    pub unk: CharId,
}

impl ControlTokens {
    /// Pinned charset_v1 control token ids.
    #[must_use]
    pub const fn pinned() -> Self {
        Self {
            bos: BOS_ID,
            eos: EOS_ID,
            unk: UNK_ID,
        }
    }
}

/// Reserved id policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReservedIdPolicy {
    /// Reject reserved id 76 when it appears in input.
    Reject,
}

/// Normalization spec instance for charset_v1.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationSpec_v1 {
    /// Schema id.
    #[serde(deserialize_with = "deserialize_normalization_schema")]
    pub schema: String,
    /// Pinned normalization order.
    #[serde(deserialize_with = "deserialize_normalization_order")]
    pub order: [&'static str; 6],
    /// Example drop threshold in percent.
    pub max_unmappable_pct_per_example: f64,
    /// Policy for reserved id 76.
    pub reserved_id_in_input_policy: ReservedIdPolicy,
    /// Self hash over canonical encoding with this field omitted.
    pub normalization_self_hash: Hash256,
}

impl NormalizationSpec_v1 {
    /// Return the pinned normalization spec.
    #[must_use]
    pub fn pinned() -> Self {
        let mut spec = Self {
            schema: NORMALIZATION_SPEC_SCHEMA_ID.to_owned(),
            order: NORMALIZATION_ORDER_V1,
            max_unmappable_pct_per_example: 2.0,
            reserved_id_in_input_policy: ReservedIdPolicy::Reject,
            normalization_self_hash: Hash256::ZERO,
        };
        spec.normalization_self_hash = spec.compute_self_hash();
        spec
    }

    /// Canonical JSON bytes including `normalization_self_hash`.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        CanonicalJson::to_vec(self).expect("normalization spec canonicalizes")
    }

    /// Self hash over canonical JSON with `normalization_self_hash` omitted.
    #[must_use]
    pub fn compute_self_hash(&self) -> Hash256 {
        self_hash_omitting_fields(Self::domain(), self, "normalization_self_hash", &[])
            .expect("normalization spec self hash canonicalizes")
    }

    /// DomainHash context for `normalization_spec.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            CRATE_NAME,
            "NormalizationSpec_v1",
            NORMALIZATION_SPEC_SCHEMA_ID,
            SCHEMA_VERSION,
        )
    }
}

/// Lexical spec instance for charset_v1.
#[allow(non_camel_case_types)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LexicalSpec_v1 {
    /// Schema id.
    #[serde(deserialize_with = "deserialize_lexical_schema")]
    pub schema: String,
    /// Ordered charset table.
    #[serde(
        serialize_with = "serialize_charset",
        deserialize_with = "deserialize_charset"
    )]
    pub charset: [Char; VOCAB_SIZE],
    /// Normalization spec.
    pub normalization: NormalizationSpec_v1,
    /// Control token ids.
    pub control_tokens: ControlTokens,
    /// Self hash over canonical encoding with this field omitted.
    pub lexical_self_hash: Hash256,
}

impl LexicalSpec_v1 {
    /// Return the pinned charset_v1 lexical spec.
    #[must_use]
    pub fn pinned() -> Self {
        let mut spec = Self {
            schema: LEXICAL_SPEC_SCHEMA_ID.to_owned(),
            charset: CHARSET_V1,
            normalization: NormalizationSpec_v1::pinned(),
            control_tokens: ControlTokens::pinned(),
            lexical_self_hash: Hash256::ZERO,
        };
        spec.lexical_self_hash = spec.compute_self_hash();
        spec
    }

    /// Canonical JSON bytes including `lexical_self_hash`.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        CanonicalJson::to_vec(self).expect("lexical spec canonicalizes")
    }

    /// Self hash over canonical JSON with `lexical_self_hash` omitted.
    #[must_use]
    pub fn compute_self_hash(&self) -> Hash256 {
        self_hash_omitting_fields(Self::domain(), self, "lexical_self_hash", &[])
            .expect("lexical spec self hash canonicalizes")
    }

    /// DomainHash context for `lexical_spec.v1`.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            CRATE_NAME,
            "LexicalSpec_v1",
            LEXICAL_SPEC_SCHEMA_ID,
            SCHEMA_VERSION,
        )
    }
}

fn deserialize_normalization_order<'de, D>(deserializer: D) -> Result<[&'static str; 6], D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    if values.len() != NORMALIZATION_ORDER_V1.len() {
        return Err(serde::de::Error::custom(
            "normalization order must have exactly 6 entries",
        ));
    }
    let mut order = [""; 6];
    for (index, value) in values.iter().enumerate() {
        let Some(step) = static_normalization_step(value) else {
            return Err(serde::de::Error::custom(format!(
                "unknown normalization step {value:?}"
            )));
        };
        if step != NORMALIZATION_ORDER_V1[index] {
            return Err(serde::de::Error::custom(
                "normalization order must match charset_v1 exactly",
            ));
        }
        order[index] = step;
    }
    Ok(order)
}

fn deserialize_lexical_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_schema_literal(deserializer, LEXICAL_SPEC_SCHEMA_ID)
}

fn deserialize_normalization_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_schema_literal(deserializer, NORMALIZATION_SPEC_SCHEMA_ID)
}

fn deserialize_schema_literal<'de, D>(
    deserializer: D,
    expected: &'static str,
) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == expected {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "expected schema id {expected:?}, got {value:?}"
        )))
    }
}

fn serialize_charset<S>(charset: &[Char; VOCAB_SIZE], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    charset.as_slice().serialize(serializer)
}

fn deserialize_charset<'de, D>(deserializer: D) -> Result<[Char; VOCAB_SIZE], D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<Char>::deserialize(deserializer)?;
    values.try_into().map_err(|values: Vec<Char>| {
        serde::de::Error::custom(format!(
            "lexical charset must have exactly {VOCAB_SIZE} entries, got {}",
            values.len()
        ))
    })
}

fn static_normalization_step(value: &str) -> Option<&'static str> {
    match value {
        "nfc" => Some("nfc"),
        "strip_combining_accents" => Some("strip_combining_accents"),
        "preserve_case" => Some("preserve_case"),
        "fold_quotes_and_dashes" => Some("fold_quotes_and_dashes"),
        "whitespace" => Some("whitespace"),
        "unmappable" => Some("unmappable"),
        _ => None,
    }
}
