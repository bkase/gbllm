//! Effective count tables for S3 modified Kneser-Ney.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use gbf_artifact::TextCharSeq;

use super::BaselineError;

/// Lexicographically ordered n-gram key.
#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct NgramKey<const K: usize> {
    tokens: [u8; K],
}

impl<const K: usize> NgramKey<K> {
    /// Build a key from a fixed-size token tuple.
    #[must_use]
    pub const fn new(tokens: [u8; K]) -> Self {
        Self { tokens }
    }

    /// Borrow the complete token tuple.
    #[must_use]
    pub const fn tokens(&self) -> &[u8; K] {
        &self.tokens
    }

    /// Borrow the context token tuple of length `K - 1`.
    #[must_use]
    pub fn context(&self) -> &[u8] {
        &self.tokens[..K.saturating_sub(1)]
    }

    /// Return the target token id.
    #[must_use]
    pub fn target(&self) -> u8 {
        self.tokens[K - 1]
    }
}

impl<const K: usize> fmt::Debug for NgramKey<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NgramKey").field(&self.tokens).finish()
    }
}

/// Fitted effective count tables consumed by `P_KN_k`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnEffectiveCounts {
    c2: BTreeMap<NgramKey<2>, u64>,
    c3: BTreeMap<NgramKey<3>, u64>,
    c4: BTreeMap<NgramKey<4>, u64>,
    c5: BTreeMap<NgramKey<5>, u64>,
    p1_left_continuations: BTreeMap<u8, u64>,
    p1_denominator: u64,
}

impl KnEffectiveCounts {
    /// Compute all S3 effective count tables from normalized train text.
    pub fn fit(corpus_train_post: &TextCharSeq) -> Result<Self, BaselineError> {
        if corpus_train_post.len() < 5 {
            return Err(BaselineError::TrainTooShort {
                min: 5,
                observed: corpus_train_post.len(),
            });
        }

        let tokens = corpus_train_post.as_slice();
        let p1_left_contexts = p1_left_contexts(tokens);
        let p1_left_continuations = p1_left_contexts
            .iter()
            .map(|(&target, lefts)| (target, lefts.len() as u64))
            .collect::<BTreeMap<_, _>>();
        let p1_denominator = p1_left_continuations.values().sum();

        Ok(Self {
            c2: c_continuation_counts::<2>(corpus_train_post),
            c3: c_continuation_counts::<3>(corpus_train_post),
            c4: c_continuation_counts::<4>(corpus_train_post),
            c5: c5_raw_counts(corpus_train_post),
            p1_left_continuations,
            p1_denominator,
        })
    }

    /// Effective count table C2.
    #[must_use]
    pub const fn c2(&self) -> &BTreeMap<NgramKey<2>, u64> {
        &self.c2
    }

    /// Effective count table C3.
    #[must_use]
    pub const fn c3(&self) -> &BTreeMap<NgramKey<3>, u64> {
        &self.c3
    }

    /// Effective count table C4.
    #[must_use]
    pub const fn c4(&self) -> &BTreeMap<NgramKey<4>, u64> {
        &self.c4
    }

    /// Effective count table C5.
    #[must_use]
    pub const fn c5(&self) -> &BTreeMap<NgramKey<5>, u64> {
        &self.c5
    }

    /// P_KN_1 numerator `N1+(•w)`.
    #[must_use]
    pub fn p1_left_continuation_count(&self, target: u8) -> u64 {
        self.p1_left_continuations
            .get(&target)
            .copied()
            .unwrap_or(0)
    }

    /// P_KN_1 denominator `N1+(••)`.
    #[must_use]
    pub const fn p1_denominator(&self) -> u64 {
        self.p1_denominator
    }

    /// Effective count for `(context, target)` at order `k`.
    pub fn count(&self, order: usize, context: &[u8], target: u8) -> Result<u64, BaselineError> {
        match order {
            2 => Ok(self
                .c2
                .get(&key_from_context_target::<2>(context, target)?)
                .copied()
                .unwrap_or(0)),
            3 => Ok(self
                .c3
                .get(&key_from_context_target::<3>(context, target)?)
                .copied()
                .unwrap_or(0)),
            4 => Ok(self
                .c4
                .get(&key_from_context_target::<4>(context, target)?)
                .copied()
                .unwrap_or(0)),
            5 => Ok(self
                .c5
                .get(&key_from_context_target::<5>(context, target)?)
                .copied()
                .unwrap_or(0)),
            _ => Err(BaselineError::InvalidOrder { order }),
        }
    }

    /// Sum of effective counts for a context at order `k`.
    pub fn context_total(&self, order: usize, context: &[u8]) -> Result<u64, BaselineError> {
        Ok(match order {
            2 => context_total(&self.c2, context)?,
            3 => context_total(&self.c3, context)?,
            4 => context_total(&self.c4, context)?,
            5 => context_total(&self.c5, context)?,
            _ => return Err(BaselineError::InvalidOrder { order }),
        })
    }

    /// Counts of target types with count 1, 2, and 3+ for a context at order `k`.
    pub fn context_count_buckets(
        &self,
        order: usize,
        context: &[u8],
    ) -> Result<(u64, u64, u64), BaselineError> {
        Ok(match order {
            2 => context_count_buckets(&self.c2, context)?,
            3 => context_count_buckets(&self.c3, context)?,
            4 => context_count_buckets(&self.c4, context)?,
            5 => context_count_buckets(&self.c5, context)?,
            _ => return Err(BaselineError::InvalidOrder { order }),
        })
    }

    /// Number of distinct entries in C_k.
    pub fn unique_count(&self, order: usize) -> Result<u64, BaselineError> {
        Ok(match order {
            2 => self.c2.len() as u64,
            3 => self.c3.len() as u64,
            4 => self.c4.len() as u64,
            5 => self.c5.len() as u64,
            _ => return Err(BaselineError::InvalidOrder { order }),
        })
    }

    /// Count-of-counts table for C_k.
    pub fn count_of_counts(&self, order: usize) -> Result<BTreeMap<u64, u64>, BaselineError> {
        Ok(match order {
            2 => count_of_counts(&self.c2),
            3 => count_of_counts(&self.c3),
            4 => count_of_counts(&self.c4),
            5 => count_of_counts(&self.c5),
            _ => return Err(BaselineError::InvalidOrder { order }),
        })
    }
}

/// Raw 5-gram counts C5.
#[must_use]
pub fn c5_raw_counts(corpus_train_post: &TextCharSeq) -> BTreeMap<NgramKey<5>, u64> {
    raw_counts::<5>(corpus_train_post.as_slice())
}

/// Modified-KN left-continuation count table C_k for k in {2,3,4}.
#[must_use]
pub fn c_continuation_counts<const K: usize>(
    corpus_train_post: &TextCharSeq,
) -> BTreeMap<NgramKey<K>, u64> {
    assert!(
        (2..=4).contains(&K),
        "C_k continuation counts are defined here only for k in {{2,3,4}}"
    );
    let mut left_contexts = BTreeMap::<NgramKey<K>, BTreeSet<u8>>::new();
    let tokens = corpus_train_post.as_slice();
    if tokens.len() < K + 1 {
        return BTreeMap::new();
    }

    for start in 1..=tokens.len() - K {
        let key = key_from_window::<K>(&tokens[start..start + K]);
        left_contexts
            .entry(key)
            .or_default()
            .insert(tokens[start - 1]);
    }

    left_contexts
        .into_iter()
        .map(|(key, lefts)| (key, lefts.len() as u64))
        .collect()
}

/// Count distinct entries by their count value.
#[must_use]
pub fn count_of_counts<const K: usize>(table: &BTreeMap<NgramKey<K>, u64>) -> BTreeMap<u64, u64> {
    let mut counts = BTreeMap::new();
    for &count in table.values() {
        *counts.entry(count).or_default() += 1;
    }
    counts
}

fn raw_counts<const K: usize>(tokens: &[u8]) -> BTreeMap<NgramKey<K>, u64> {
    let mut counts = BTreeMap::new();
    if tokens.len() < K {
        return counts;
    }
    for window in tokens.windows(K) {
        *counts.entry(key_from_window::<K>(window)).or_default() += 1;
    }
    counts
}

fn p1_left_contexts(tokens: &[u8]) -> BTreeMap<u8, BTreeSet<u8>> {
    let mut left_contexts = BTreeMap::<u8, BTreeSet<u8>>::new();
    for window in tokens.windows(2) {
        left_contexts
            .entry(window[1])
            .or_default()
            .insert(window[0]);
    }
    left_contexts
}

fn key_from_context_target<const K: usize>(
    context: &[u8],
    target: u8,
) -> Result<NgramKey<K>, BaselineError> {
    if context.len() != K - 1 {
        return Err(BaselineError::InvalidOrder {
            order: context.len() + 1,
        });
    }
    let mut tokens = [0_u8; K];
    tokens[..K - 1].copy_from_slice(context);
    tokens[K - 1] = target;
    Ok(NgramKey::new(tokens))
}

fn key_from_window<const K: usize>(window: &[u8]) -> NgramKey<K> {
    let mut tokens = [0_u8; K];
    tokens.copy_from_slice(window);
    NgramKey::new(tokens)
}

fn context_total<const K: usize>(
    table: &BTreeMap<NgramKey<K>, u64>,
    context: &[u8],
) -> Result<u64, BaselineError> {
    if context.len() != K - 1 {
        return Err(BaselineError::InvalidOrder {
            order: context.len() + 1,
        });
    }
    Ok(table
        .iter()
        .filter(|(key, _)| key.context() == context)
        .map(|(_, count)| *count)
        .sum())
}

fn context_count_buckets<const K: usize>(
    table: &BTreeMap<NgramKey<K>, u64>,
    context: &[u8],
) -> Result<(u64, u64, u64), BaselineError> {
    if context.len() != K - 1 {
        return Err(BaselineError::InvalidOrder {
            order: context.len() + 1,
        });
    }
    let mut n1 = 0;
    let mut n2 = 0;
    let mut n3p = 0;
    for (key, count) in table.iter().filter(|(key, _)| key.context() == context) {
        debug_assert_eq!(key.context(), context);
        match *count {
            1 => n1 += 1,
            2 => n2 += 1,
            3.. => n3p += 1,
            0 => {}
        }
    }
    Ok((n1, n2, n3p))
}
