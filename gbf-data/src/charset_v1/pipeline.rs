//! S3 `charset_v1` corpus operation.

use gbf_artifact::{LexicalSpec_v1, TextCharSeq};
use gbf_foundation::{CanonicalJson, DomainHash, Hash256, sha256};
use serde::{Deserialize, Deserializer, Serialize};

use super::normalize_raw::{CharsetError, DropReason, normalize_raw};

const CHARSET_PRODUCT_SCHEMA: &str = "s3_charset_v1.v1";
const CHARSET_PRODUCT_SCHEMA_VERSION: &str = "1";

/// Inputs to the S3 charset operation.
#[derive(Debug, Clone, PartialEq)]
pub struct CharsetInputs {
    pub raw_train_examples: Vec<Vec<u8>>,
    pub raw_val_examples: Vec<Vec<u8>>,
    pub spec: LexicalSpec_v1,
}

/// S3 charset output record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharsetProduct {
    #[serde(deserialize_with = "deserialize_charset_product_schema")]
    pub schema: String,
    pub train_post: TextCharSeq,
    pub val_post: TextCharSeq,
    pub train_post_sha256: Hash256,
    pub val_post_sha256: Hash256,
    pub charset_v1_sha256: Hash256,
    pub unmappable_example_drop_rate_train: f64,
    pub unmappable_example_drop_rate_val: f64,
    pub unmappable_char_drop_rate_train: f64,
    pub unmappable_char_drop_rate_val: f64,
    pub drop_log: Vec<DropEvent>,
    pub charset_self_hash: Hash256,
}

/// Split label for drop events.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharsetSplit {
    Train,
    Val,
}

/// One dropped example event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DropEvent {
    pub split: CharsetSplit,
    pub example_id: u64,
    pub byte_count: u64,
    pub post_token_count: u64,
    pub unk_count: u32,
    pub drop_reason: DropReasonRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReasonRecord {
    UnmappableFractionExceeded,
}

impl From<DropReason> for DropReasonRecord {
    fn from(value: DropReason) -> Self {
        match value {
            DropReason::UnmappableFractionExceeded => Self::UnmappableFractionExceeded,
        }
    }
}

/// Run S3 charset normalization for train and validation examples.
pub fn s3_charset_v1(inputs: CharsetInputs) -> Result<CharsetProduct, CharsetError> {
    if inputs.spec != LexicalSpec_v1::pinned() {
        return Err(CharsetError::SpecMismatch);
    }

    let raw_train_byte_count = sum_byte_count(&inputs.raw_train_examples);
    let raw_val_byte_count = sum_byte_count(&inputs.raw_val_examples);
    let charset_v1_sha256 = inputs.spec.lexical_self_hash;
    tracing::info!(
        target: "gbf_data::charset_v1",
        event_name = "s3::charset::pipeline_started",
        raw_train_byte_count,
        raw_val_byte_count,
        charset_v1_sha256 = %charset_v1_sha256,
    );

    let mut drop_log = Vec::new();
    let train = normalize_split(
        CharsetSplit::Train,
        &inputs.raw_train_examples,
        &mut drop_log,
    )?;
    let val = normalize_split(CharsetSplit::Val, &inputs.raw_val_examples, &mut drop_log)?;
    let unmappable_example_drop_rate_train = train.example_drop_rate();
    let unmappable_example_drop_rate_val = val.example_drop_rate();
    let unmappable_char_drop_rate_train = train.char_drop_rate();
    let unmappable_char_drop_rate_val = val.char_drop_rate();

    let mut product = CharsetProduct {
        schema: CHARSET_PRODUCT_SCHEMA.to_owned(),
        train_post_sha256: sha256(train.post_tokens.as_slice()),
        val_post_sha256: sha256(val.post_tokens.as_slice()),
        train_post: TextCharSeq::new(train.post_tokens).map_err(CharsetError::Lexical)?,
        val_post: TextCharSeq::new(val.post_tokens).map_err(CharsetError::Lexical)?,
        charset_v1_sha256,
        unmappable_example_drop_rate_train,
        unmappable_example_drop_rate_val,
        unmappable_char_drop_rate_train,
        unmappable_char_drop_rate_val,
        drop_log,
        charset_self_hash: Hash256::ZERO,
    };
    product.charset_self_hash = product.compute_self_hash()?;

    tracing::info!(
        target: "gbf_data::charset_v1",
        event_name = "s3::charset::pipeline_complete",
        train_post_char_count = product.train_post.len(),
        val_post_char_count = product.val_post.len(),
        unmappable_example_drop_rate_train = product.unmappable_example_drop_rate_train,
        unmappable_example_drop_rate_val = product.unmappable_example_drop_rate_val,
        unmappable_char_drop_rate_train = product.unmappable_char_drop_rate_train,
        unmappable_char_drop_rate_val = product.unmappable_char_drop_rate_val,
        charset_self_hash = %product.charset_self_hash,
    );

    Ok(product)
}

impl CharsetProduct {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, CharsetError> {
        CanonicalJson::to_vec(self).map_err(CharsetError::CanonicalJson)
    }

    pub fn compute_self_hash(&self) -> Result<Hash256, CharsetError> {
        let mut value = serde_json::to_value(self).map_err(CharsetError::SerdeJson)?;
        value
            .as_object_mut()
            .ok_or(CharsetError::ExpectedObjectForSelfHash)?
            .remove("charset_self_hash");
        let canonical = CanonicalJson::value_to_vec(&value).map_err(CharsetError::CanonicalJson)?;
        DomainHash::new(
            "gbf-data",
            "CharsetProduct",
            CHARSET_PRODUCT_SCHEMA,
            CHARSET_PRODUCT_SCHEMA_VERSION,
        )
        .hash_canonical_bytes(&canonical)
        .map_err(CharsetError::CanonicalJson)
    }
}

/// Verify a normalized char sequence against an expected SHA-256.
pub fn verify_charseq_sha256(seq: &TextCharSeq, expected: Hash256) -> Result<(), CharsetError> {
    let observed = sha256(seq.as_slice());
    if observed == expected {
        Ok(())
    } else {
        Err(CharsetError::PostShaMismatch { expected, observed })
    }
}

#[derive(Debug, Clone, Default)]
struct SplitAccum {
    post_tokens: Vec<u8>,
    total_examples: u64,
    dropped_examples: u64,
    pre_drop_token_count: u64,
    dropped_token_count: u64,
}

impl SplitAccum {
    fn example_drop_rate(&self) -> f64 {
        if self.total_examples == 0 {
            0.0
        } else {
            self.dropped_examples as f64 / self.total_examples as f64
        }
    }

    fn char_drop_rate(&self) -> f64 {
        if self.pre_drop_token_count == 0 {
            0.0
        } else {
            self.dropped_token_count as f64 / self.pre_drop_token_count as f64
        }
    }
}

fn normalize_split(
    split: CharsetSplit,
    examples: &[Vec<u8>],
    drop_log: &mut Vec<DropEvent>,
) -> Result<SplitAccum, CharsetError> {
    let mut accum = SplitAccum {
        total_examples: examples.len() as u64,
        ..SplitAccum::default()
    };

    for (example_id, raw) in examples.iter().enumerate() {
        let stats = normalize_raw(raw)?;
        let post_token_count = stats.tokens.len() as u64;
        accum.pre_drop_token_count += post_token_count;
        tracing::trace!(
            target: "gbf_data::charset_v1",
            event_name = "s3::charset::example_normalized",
            example_id = example_id as u64,
            byte_count = raw.len() as u64,
            post_token_count,
            unk_count = stats.unk_count_in_example,
            dropped = stats.dropped,
            drop_reason = ?stats.drop_reason,
        );
        if stats.dropped {
            accum.dropped_examples += 1;
            accum.dropped_token_count += post_token_count;
            drop_log.push(DropEvent {
                split,
                example_id: example_id as u64,
                byte_count: raw.len() as u64,
                post_token_count,
                unk_count: stats.unk_count_in_example,
                drop_reason: stats
                    .drop_reason
                    .expect("dropped examples carry a drop reason")
                    .into(),
            });
        } else {
            accum.post_tokens.extend_from_slice(stats.tokens.as_slice());
        }
    }

    Ok(accum)
}

fn sum_byte_count(examples: &[Vec<u8>]) -> u64 {
    examples.iter().map(|example| example.len() as u64).sum()
}

fn deserialize_charset_product_schema<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == CHARSET_PRODUCT_SCHEMA {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format_args!(
            "expected schema id {CHARSET_PRODUCT_SCHEMA:?}, got {value:?}"
        )))
    }
}
