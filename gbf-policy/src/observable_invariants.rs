//! Observable invariant helpers for F-S5 closure checks.

use std::collections::{BTreeMap, BTreeSet};

use gbf_foundation::Hash256;
use serde::{Deserialize, Serialize};

pub const S5_SEEDS: [u64; 5] = [0, 1, 2, 3, 4];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum S5VariantId {
    #[serde(rename = "BoundedKv")]
    BoundedKv,
    #[serde(rename = "L_FIX1")]
    LFix1,
    #[serde(rename = "L_MT4")]
    LMt4,
}

impl S5VariantId {
    pub const ALL: [Self; 3] = [Self::BoundedKv, Self::LFix1, Self::LMt4];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O9SafetensorsHashObservation {
    pub variant: S5VariantId,
    pub seed: u64,
    pub safetensors_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O9VariantResult {
    pub variant: S5VariantId,
    pub observed_seed_count: usize,
    pub distinct_safetensors_hash_count: usize,
    pub passes: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct O9CrossSeedDifferenceReport {
    pub passes: bool,
    pub variant_results: Vec<O9VariantResult>,
}

#[must_use]
pub fn o9_cross_seed_difference_report(
    observations: &[O9SafetensorsHashObservation],
) -> O9CrossSeedDifferenceReport {
    let allowed_seeds = BTreeSet::from(S5_SEEDS);
    let mut by_variant = BTreeMap::<S5VariantId, BTreeMap<u64, Hash256>>::new();

    for observation in observations {
        if allowed_seeds.contains(&observation.seed) {
            by_variant
                .entry(observation.variant)
                .or_default()
                .insert(observation.seed, observation.safetensors_hash);
        }
    }

    let variant_results = S5VariantId::ALL
        .into_iter()
        .map(|variant| {
            let observed = by_variant.get(&variant);
            let distinct_safetensors_hash_count = observed
                .map(|hashes_by_seed| {
                    hashes_by_seed
                        .values()
                        .copied()
                        .collect::<BTreeSet<_>>()
                        .len()
                })
                .unwrap_or(0);
            O9VariantResult {
                variant,
                observed_seed_count: observed.map(BTreeMap::len).unwrap_or(0),
                distinct_safetensors_hash_count,
                passes: distinct_safetensors_hash_count >= 2,
            }
        })
        .collect::<Vec<_>>();

    O9CrossSeedDifferenceReport {
        passes: variant_results.iter().all(|result| result.passes),
        variant_results,
    }
}

#[must_use]
pub fn o9_cross_seed_difference_passes(observations: &[O9SafetensorsHashObservation]) -> bool {
    o9_cross_seed_difference_report(observations).passes
}
