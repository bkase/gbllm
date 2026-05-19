//! End-to-end S3 KN baseline fitting and scoring.

use std::collections::BTreeMap;

use gbf_artifact::TextCharSeq;
use gbf_foundation::{DomainHash, Hash256, self_hash_omitting_fields, sha256};
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize};

use super::canonical_counts_write::CanonicalKnCountsWrite;
use super::conditional::KnConditionalModel;
use super::discounts::{KnDiscounts, fit_discounts_for_order};
use super::kn_effective_counts::KnEffectiveCounts;
use super::{BaselineError, KN_MAX_ORDER, KN_RESET_CHUNK_SIZE, S3_BASELINE_LOG_TARGET};

const BASELINE_SCHEMA: &str = "s3_baseline_kn5.v1";
const BASELINE_SCHEMA_VERSION: &str = "1";

/// Inputs to the S3 KN baseline operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnBaselineInputs {
    /// Normalized training character stream.
    pub train_post: TextCharSeq,
    /// Normalized validation character stream.
    pub val_post: TextCharSeq,
}

/// Count cardinalities emitted with `s3_baseline_kn5.v1`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountsSummary {
    /// Number of normalized train characters.
    pub train_chars: u64,
    /// Number of normalized validation characters.
    pub val_chars: u64,
    /// Number of distinct C2 entries.
    pub c2_unique_count: u64,
    /// Number of distinct C3 entries.
    pub c3_unique_count: u64,
    /// Number of distinct C4 entries.
    pub c4_unique_count: u64,
    /// Number of distinct C5 entries.
    pub c5_unique_count: u64,
}

/// `s3_baseline_kn5.v1` product.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnBaselineProduct {
    /// Schema id.
    pub schema: String,
    /// SHA-256 of train_post bytes.
    pub train_post_sha256: Hash256,
    /// SHA-256 of val_post bytes.
    pub val_post_sha256: Hash256,
    /// Pinned max order, always 5.
    pub order: u64,
    /// Per-order D-rule discounts for k in {2,3,4,5}.
    #[serde(
        serialize_with = "serialize_discount_map",
        deserialize_with = "deserialize_discount_map"
    )]
    pub discounts: BTreeMap<u64, KnDiscounts>,
    /// P_KN_1 reset-context validation bpc.
    pub bpc_kn1_val: f64,
    /// P_KN_2 reset-context validation bpc.
    pub bpc_kn2_val: f64,
    /// P_KN_3 reset-context validation bpc.
    pub bpc_kn3_val: f64,
    /// P_KN_4 reset-context validation bpc.
    pub bpc_kn4_val: f64,
    /// P_KN_5 reset-context validation bpc.
    pub bpc_kn5_val: f64,
    /// Count cardinality summary.
    pub counts_summary: CountsSummary,
    /// Hash of CanonicalKnCountsWrite bytes.
    pub counts_blob_sha256: Hash256,
    /// Self-hash over this report with this field omitted.
    pub baseline_self_hash: Hash256,
}

impl KnBaselineProduct {
    /// DomainHash context.
    #[must_use]
    pub const fn domain() -> DomainHash<'static> {
        DomainHash::new(
            "gbf-experiments",
            "KnBaselineProduct",
            BASELINE_SCHEMA,
            BASELINE_SCHEMA_VERSION,
        )
    }

    /// Compute self-hash with `baseline_self_hash` omitted.
    pub fn computed_self_hash(&self) -> Result<Hash256, BaselineError> {
        Ok(self_hash_omitting_fields(
            Self::domain(),
            self,
            "baseline_self_hash",
            &[],
        )?)
    }

    /// Return a product with `baseline_self_hash` recomputed.
    pub fn with_computed_self_hash(mut self) -> Result<Self, BaselineError> {
        self.baseline_self_hash = self.computed_self_hash()?;
        Ok(self)
    }
}

/// Fit and score the pinned S3 5-gram modified Kneser-Ney baseline.
pub fn s3_fit_kn5(inputs: KnBaselineInputs) -> Result<KnBaselineProduct, BaselineError> {
    if inputs.train_post.len() < KN_MAX_ORDER {
        return Err(BaselineError::TrainTooShort {
            min: KN_MAX_ORDER,
            observed: inputs.train_post.len(),
        });
    }
    if inputs.val_post.is_empty() {
        return Err(BaselineError::EmptyValidation);
    }

    tracing::info!(
        target: S3_BASELINE_LOG_TARGET,
        event_name = "s3::baseline::fit_started",
        train_post_char_count = inputs.train_post.len() as u64,
        vocab_size = 80_u64,
        order = 5_u64,
    );

    let counts = KnEffectiveCounts::fit(&inputs.train_post)?;
    tracing::info!(
        target: S3_BASELINE_LOG_TARGET,
        event_name = "s3::baseline::counts_computed",
        c5_unique_count = counts.unique_count(5)?,
        c4_unique_count = counts.unique_count(4)?,
        c3_unique_count = counts.unique_count(3)?,
        c2_unique_count = counts.unique_count(2)?,
    );

    let discounts = match fit_all_discounts(&counts) {
        Ok(discounts) => discounts,
        Err(error) => {
            if let BaselineError::DiscountPreconditionsViolated { order, missing } = &error {
                tracing::info!(
                    target: S3_BASELINE_LOG_TARGET,
                    event_name = "s3::baseline::aborted",
                    reason = "discount_preconditions_violated",
                    order = *order,
                    missing_count_of_counts = ?missing,
                );
            }
            return Err(error);
        }
    };

    let d_1_per_order = discount_array(&discounts, |d| d.d_1);
    let d_2_per_order = discount_array(&discounts, |d| d.d_2);
    let d_3p_per_order = discount_array(&discounts, |d| d.d_3p);
    let y_per_order = discount_array(&discounts, |d| d.y_k);
    tracing::info!(
        target: S3_BASELINE_LOG_TARGET,
        event_name = "s3::baseline::discounts_fit",
        d_1_per_order = ?d_1_per_order,
        d_2_per_order = ?d_2_per_order,
        d_3p_per_order = ?d_3p_per_order,
        y_per_order = ?y_per_order,
        d_1_order_2 = d_1_per_order[0],
        d_1_order_3 = d_1_per_order[1],
        d_1_order_4 = d_1_per_order[2],
        d_1_order_5 = d_1_per_order[3],
        d_2_order_2 = d_2_per_order[0],
        d_2_order_3 = d_2_per_order[1],
        d_2_order_4 = d_2_per_order[2],
        d_2_order_5 = d_2_per_order[3],
        d_3p_order_2 = d_3p_per_order[0],
        d_3p_order_3 = d_3p_per_order[1],
        d_3p_order_4 = d_3p_per_order[2],
        d_3p_order_5 = d_3p_per_order[3],
        y_order_2 = y_per_order[0],
        y_order_3 = y_per_order[1],
        y_order_4 = y_per_order[2],
        y_order_5 = y_per_order[3],
    );

    let model = KnConditionalModel::new(counts.clone(), discounts.clone());
    let bpc_kn1_val = bpc_for_order(&model, 1, inputs.val_post.as_slice())?;
    let bpc_kn2_val = bpc_for_order(&model, 2, inputs.val_post.as_slice())?;
    let bpc_kn3_val = bpc_for_order(&model, 3, inputs.val_post.as_slice())?;
    let bpc_kn4_val = bpc_for_order(&model, 4, inputs.val_post.as_slice())?;
    let bpc_kn5_val = bpc_for_order(&model, 5, inputs.val_post.as_slice())?;
    let counts_summary = CountsSummary {
        train_chars: inputs.train_post.len() as u64,
        val_chars: inputs.val_post.len() as u64,
        c2_unique_count: counts.unique_count(2)?,
        c3_unique_count: counts.unique_count(3)?,
        c4_unique_count: counts.unique_count(4)?,
        c5_unique_count: counts.unique_count(5)?,
    };
    let counts_blob_sha256 = CanonicalKnCountsWrite::sha256(&counts);

    let product = KnBaselineProduct {
        schema: BASELINE_SCHEMA.to_owned(),
        train_post_sha256: sha256(inputs.train_post.as_slice()),
        val_post_sha256: sha256(inputs.val_post.as_slice()),
        order: KN_MAX_ORDER as u64,
        discounts,
        bpc_kn1_val,
        bpc_kn2_val,
        bpc_kn3_val,
        bpc_kn4_val,
        bpc_kn5_val,
        counts_summary,
        counts_blob_sha256,
        baseline_self_hash: Hash256::ZERO,
    }
    .with_computed_self_hash()?;

    tracing::info!(
        target: S3_BASELINE_LOG_TARGET,
        event_name = "s3::baseline::scoring_complete",
        bpc_kn5_val = product.bpc_kn5_val,
        bpc_kn4_val = product.bpc_kn4_val,
        bpc_kn3_val = product.bpc_kn3_val,
        bpc_kn2_val = product.bpc_kn2_val,
        bpc_kn1_val = product.bpc_kn1_val,
        baseline_self_hash = %product.baseline_self_hash,
        counts_blob_sha256 = %product.counts_blob_sha256,
    );

    Ok(product)
}

fn fit_all_discounts(
    counts: &KnEffectiveCounts,
) -> Result<BTreeMap<u64, KnDiscounts>, BaselineError> {
    let mut discounts = BTreeMap::new();
    for order in 2..=5 {
        let coc = counts.count_of_counts(order)?;
        discounts.insert(order as u64, fit_discounts_for_order(order as u64, &coc)?);
    }
    Ok(discounts)
}

fn bpc_for_order(
    model: &KnConditionalModel,
    max_order: usize,
    val: &[u8],
) -> Result<f64, BaselineError> {
    if val.is_empty() {
        return Err(BaselineError::EmptyValidation);
    }
    let mut log2_sum = 0.0;
    for chunk in val.chunks(KN_RESET_CHUNK_SIZE) {
        for (chunk_offset, &target) in chunk.iter().enumerate() {
            let order = (chunk_offset + 1).min(max_order);
            let context_start = chunk_offset.saturating_sub(order.saturating_sub(1));
            let context = &chunk[context_start..chunk_offset];
            let probability = model.probability(order, context, target)?.get();
            if probability <= 0.0 {
                return Err(BaselineError::ZeroProbability {
                    order,
                    chunk_offset,
                    target,
                });
            }
            log2_sum -= probability.log2();
        }
    }
    Ok(log2_sum / val.len() as f64)
}

fn discount_array(
    discounts: &BTreeMap<u64, KnDiscounts>,
    f: impl Fn(&KnDiscounts) -> f64,
) -> [f64; 4] {
    [2_u64, 3, 4, 5].map(|order| f(discounts.get(&order).expect("discount order fitted")))
}

fn serialize_discount_map<S>(
    discounts: &BTreeMap<u64, KnDiscounts>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut map = serializer.serialize_map(Some(discounts.len()))?;
    for (order, discounts) in discounts {
        map.serialize_entry(&order.to_string(), discounts)?;
    }
    map.end()
}

fn deserialize_discount_map<'de, D>(deserializer: D) -> Result<BTreeMap<u64, KnDiscounts>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = BTreeMap::<String, KnDiscounts>::deserialize(deserializer)?;
    let mut discounts = BTreeMap::new();
    for (order, value) in raw {
        let order = order.parse::<u64>().map_err(serde::de::Error::custom)?;
        discounts.insert(order, value);
    }
    Ok(discounts)
}
