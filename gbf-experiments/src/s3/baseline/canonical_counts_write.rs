//! CanonicalKnCountsWrite encoder.

use gbf_foundation::{Hash256, sha256};

use super::kn_effective_counts::{KnEffectiveCounts, NgramKey};

/// Deterministic encoder for effective KN count tables C2..C5.
#[derive(Debug, Clone, Copy, Default)]
pub struct CanonicalKnCountsWrite;

impl CanonicalKnCountsWrite {
    /// Encode all effective count tables in RFC order.
    #[must_use]
    pub fn encode(counts: &KnEffectiveCounts) -> Vec<u8> {
        Self::encode_tables(counts.c2(), counts.c3(), counts.c4(), counts.c5())
    }

    /// Encode explicit effective count tables in RFC order.
    #[must_use]
    pub fn encode_tables(
        c2: &std::collections::BTreeMap<NgramKey<2>, u64>,
        c3: &std::collections::BTreeMap<NgramKey<3>, u64>,
        c4: &std::collections::BTreeMap<NgramKey<4>, u64>,
        c5: &std::collections::BTreeMap<NgramKey<5>, u64>,
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        encode_table::<2>(&mut bytes, 2, c2);
        encode_table::<3>(&mut bytes, 3, c3);
        encode_table::<4>(&mut bytes, 4, c4);
        encode_table::<5>(&mut bytes, 5, c5);
        bytes
    }

    /// Hash the canonical count blob.
    #[must_use]
    pub fn sha256(counts: &KnEffectiveCounts) -> Hash256 {
        sha256(Self::encode(counts))
    }
}

fn encode_table<const K: usize>(
    bytes: &mut Vec<u8>,
    order: u8,
    table: &std::collections::BTreeMap<NgramKey<K>, u64>,
) {
    for (key, count) in table {
        bytes.push(order);
        bytes.extend_from_slice(key.context());
        bytes.push(key.target());
        bytes.extend_from_slice(&count.to_le_bytes());
    }
}
