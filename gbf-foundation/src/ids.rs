//! Typed identifiers shared across crate boundaries.
//!
//! ```compile_fail
//! use gbf_foundation::{ExpertId, LayerId};
//!
//! fn takes_layer(_: LayerId) {}
//!
//! takes_layer(ExpertId::from(1));
//! ```

use std::fmt;

use serde::{Deserialize, Serialize};

macro_rules! numeric_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(u16);

        impl $name {
            #[must_use]
            pub const fn new(value: u16) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn get(self) -> u16 {
                self.0
            }
        }

        impl From<u16> for $name {
            fn from(value: u16) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for u16 {
            fn from(value: $name) -> Self {
                value.get()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(String);

        impl $name {
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.into_string()
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_id!(TargetProfileId);
string_id!(CompileProfileId);
string_id!(TargetFamilyId);
string_id!(CheckpointId);
string_id!(WorkloadId);
string_id!(CalibrationSetRef);

numeric_id!(LayerId);
numeric_id!(ExpertId);
numeric_id!(BudgetSlotId);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_ids_preserve_type_and_value() {
        let layer = LayerId::from(7);
        let expert = ExpertId::new(7);
        let slot = BudgetSlotId::new(3);

        assert_eq!(layer.get(), 7);
        assert_eq!(expert.get(), 7);
        assert_eq!(slot.to_string(), "3");
    }

    #[test]
    fn string_ids_preserve_type_and_value() {
        let target = TargetProfileId::from("gb-color");
        let profile = CompileProfileId::new("bringup");
        let family = TargetFamilyId::from(String::from("lr35902"));

        assert_eq!(target.as_str(), "gb-color");
        assert_eq!(profile.to_string(), "bringup");
        assert_eq!(family.into_string(), "lr35902");
    }

    #[test]
    fn ids_round_trip_through_serde() {
        let encoded = serde_json::to_string(&WorkloadId::from("smoke")).expect("id serializes");
        let decoded: WorkloadId = serde_json::from_str(&encoded).expect("id deserializes");

        assert_eq!(decoded, WorkloadId::from("smoke"));

        let encoded = serde_json::to_string(&LayerId::new(12)).expect("layer id serializes");
        let decoded: LayerId = serde_json::from_str(&encoded).expect("layer id deserializes");

        assert_eq!(decoded, LayerId::new(12));
    }
}
