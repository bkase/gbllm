//! Compile-profile policy selectors.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SequenceSemanticsRef {
    #[default]
    Unspecified,
    LinearState,
    BoundedKv,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_semantics_ref_defaults_to_unspecified_until_profiles_are_defined() {
        assert_eq!(
            SequenceSemanticsRef::default(),
            SequenceSemanticsRef::Unspecified
        );
    }

    #[test]
    fn sequence_semantics_ref_round_trips_through_serde() {
        let encoded = serde_json::to_string(&SequenceSemanticsRef::BoundedKv).unwrap();
        let decoded: SequenceSemanticsRef = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded, SequenceSemanticsRef::BoundedKv);
    }
}
