//! Typed identifiers shared across crate boundaries.
//!
//! ```compile_fail
//! use gbf_foundation::{ExpertId, LayerId};
//!
//! fn takes_layer(_: LayerId) {}
//!
//! takes_layer(ExpertId::from(1));
//! ```

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone)]
enum StringIdValue {
    Static(&'static str),
    Owned(String),
}

impl StringIdValue {
    #[must_use]
    pub const fn from_static(value: &'static str) -> Self {
        Self::Static(value)
    }

    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self::Owned(value.into())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::Static(value) => value,
            Self::Owned(value) => value.as_str(),
        }
    }

    #[must_use]
    pub fn into_string(self) -> String {
        match self {
            Self::Static(value) => value.to_owned(),
            Self::Owned(value) => value,
        }
    }
}

impl PartialEq for StringIdValue {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for StringIdValue {}

impl PartialOrd for StringIdValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for StringIdValue {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl Hash for StringIdValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl Serialize for StringIdValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for StringIdValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::Owned)
    }
}

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
        pub struct $name(StringIdValue);

        impl $name {
            #[must_use]
            pub const fn from_static(value: &'static str) -> Self {
                Self(StringIdValue::from_static(value))
            }

            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(StringIdValue::new(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                self.0.as_str()
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0.into_string()
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
string_id!(PlatformCalibrationId);
string_id!(KernelCalibrationId);
string_id!(RuntimeCalibrationId);
string_id!(CalibrationCohortId);
string_id!(KernelImplId);
string_id!(RuntimeNucleusId);
string_id!(KernelSpecId);

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
        const TARGET: TargetProfileId = TargetProfileId::from_static("gb-color");
        let target = TARGET;
        let profile = CompileProfileId::new("bringup");
        let family = TargetFamilyId::from(String::from("lr35902"));
        let platform = PlatformCalibrationId::from("platform-dmg-001");

        assert_eq!(target.as_str(), "gb-color");
        assert_eq!(profile.to_string(), "bringup");
        assert_eq!(family.into_string(), "lr35902");
        assert_eq!(platform.as_str(), "platform-dmg-001");
        assert_eq!(TargetProfileId::from("gb-color"), target);
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
