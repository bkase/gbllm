//! Registered model size profiles for policy-side shape selection.

use std::error::Error;
use std::fmt;

use gbf_foundation::ByteCost;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ModelSizeProfile {
    Toy0,
    Toy1,
    #[cfg(feature = "falsify")]
    ToyTiny,
    MoeTiny {
        n_experts: MoeTinyExpertCount,
    },
    UpperBankCandidate {
        d_model: UpperBankDModel,
        n_experts: UpperBankExpertCount,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoeTinyExpertCount(u8);

impl MoeTinyExpertCount {
    pub fn new(n_experts: u8) -> Result<Self, ModelSizeProfileError> {
        validate_moe_tiny_experts(n_experts)?;
        Ok(Self(n_experts))
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpperBankDModel(u16);

impl UpperBankDModel {
    pub fn new(d_model: u16) -> Result<Self, ModelSizeProfileError> {
        validate_upper_bank_d_model(d_model)?;
        Ok(Self(d_model))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpperBankExpertCount(u8);

impl UpperBankExpertCount {
    pub fn new(n_experts: u8) -> Result<Self, ModelSizeProfileError> {
        validate_upper_bank_experts(n_experts)?;
        Ok(Self(n_experts))
    }

    #[must_use]
    pub const fn get(self) -> u8 {
        self.0
    }
}

impl ModelSizeProfile {
    pub const TOY0_D_MODEL: u16 = 16;
    pub const TOY0_D_FF: u16 = 32;
    pub const TOY0_N_BLOCKS: u8 = 1;
    pub const TOY1_D_MODEL: u16 = 32;
    pub const TOY1_D_FF: u16 = 64;
    pub const TOY1_N_BLOCKS: u8 = 2;
    #[cfg(feature = "falsify")]
    pub const TOY_TINY_D_MODEL: u16 = 2;
    #[cfg(feature = "falsify")]
    pub const TOY_TINY_D_FF: u16 = 4;
    #[cfg(feature = "falsify")]
    pub const TOY_TINY_N_BLOCKS: u8 = 1;
    pub const MOE_TINY_D_MODEL: u16 = 64;
    pub const MOE_TINY_D_FF: u16 = 128;
    pub const MOE_TINY_N_BLOCKS: u8 = 4;
    pub const UPPER_BANK_D_FF: u16 = 192;
    pub const UPPER_BANK_N_BLOCKS: u8 = 4;
    pub const UPPER_BANK_N_EXPERTS: u8 = 4;

    #[must_use]
    pub const fn toy0() -> Self {
        Self::Toy0
    }

    #[must_use]
    pub const fn toy1() -> Self {
        Self::Toy1
    }

    #[cfg(feature = "falsify")]
    #[must_use]
    pub const fn toy_tiny_for_falsification() -> Self {
        Self::ToyTiny
    }

    pub fn moe_tiny(n_experts: u8) -> Result<Self, ModelSizeProfileError> {
        Ok(Self::MoeTiny {
            n_experts: MoeTinyExpertCount::new(n_experts)?,
        })
    }

    pub fn upper_bank_candidate(
        d_model: u16,
        n_experts: u8,
    ) -> Result<Self, ModelSizeProfileError> {
        Self::upper_bank_candidate_with_dims(d_model, Self::UPPER_BANK_D_FF, n_experts)
    }

    pub fn upper_bank_candidate_with_dims(
        d_model: u16,
        d_ff: u16,
        n_experts: u8,
    ) -> Result<Self, ModelSizeProfileError> {
        validate_upper_bank_dims(d_model, d_ff)?;
        Ok(Self::UpperBankCandidate {
            d_model: UpperBankDModel::new(d_model)?,
            n_experts: UpperBankExpertCount::new(n_experts)?,
        })
    }

    #[must_use]
    pub const fn d_model(self) -> u16 {
        match self {
            Self::Toy0 => Self::TOY0_D_MODEL,
            Self::Toy1 => Self::TOY1_D_MODEL,
            #[cfg(feature = "falsify")]
            Self::ToyTiny => Self::TOY_TINY_D_MODEL,
            Self::MoeTiny { .. } => Self::MOE_TINY_D_MODEL,
            Self::UpperBankCandidate { d_model, .. } => d_model.get(),
        }
    }

    #[must_use]
    pub const fn d_ff(self) -> u16 {
        match self {
            Self::Toy0 => Self::TOY0_D_FF,
            Self::Toy1 => Self::TOY1_D_FF,
            #[cfg(feature = "falsify")]
            Self::ToyTiny => Self::TOY_TINY_D_FF,
            Self::MoeTiny { .. } => Self::MOE_TINY_D_FF,
            Self::UpperBankCandidate { .. } => Self::UPPER_BANK_D_FF,
        }
    }

    #[must_use]
    pub const fn n_blocks(self) -> u8 {
        match self {
            Self::Toy0 => Self::TOY0_N_BLOCKS,
            Self::Toy1 => Self::TOY1_N_BLOCKS,
            #[cfg(feature = "falsify")]
            Self::ToyTiny => Self::TOY_TINY_N_BLOCKS,
            Self::MoeTiny { .. } => Self::MOE_TINY_N_BLOCKS,
            Self::UpperBankCandidate { .. } => Self::UPPER_BANK_N_BLOCKS,
        }
    }

    #[must_use]
    pub const fn n_experts(self) -> u8 {
        match self {
            Self::Toy0 | Self::Toy1 => 0,
            #[cfg(feature = "falsify")]
            Self::ToyTiny => 0,
            Self::MoeTiny { n_experts } => n_experts.get(),
            Self::UpperBankCandidate { n_experts, .. } => n_experts.get(),
        }
    }

    #[must_use]
    pub fn expert_byte_cost(self) -> ByteCost {
        let d_model = u32::from(self.d_model());
        let d_ff = u32::from(self.d_ff());

        default_expert_matrix_byte_cost(d_ff, d_model)
            + default_expert_matrix_byte_cost(d_model, d_ff)
    }
}

impl Serialize for ModelSizeProfile {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        #[derive(Serialize)]
        enum ModelSizeProfileSerde {
            Toy0,
            Toy1,
            #[cfg(feature = "falsify")]
            ToyTiny,
            MoeTiny {
                n_experts: u8,
            },
            UpperBankCandidate {
                d_model: u16,
                n_experts: u8,
            },
        }

        let raw = match *self {
            Self::Toy0 => ModelSizeProfileSerde::Toy0,
            Self::Toy1 => ModelSizeProfileSerde::Toy1,
            #[cfg(feature = "falsify")]
            Self::ToyTiny => ModelSizeProfileSerde::ToyTiny,
            Self::MoeTiny { n_experts } => ModelSizeProfileSerde::MoeTiny {
                n_experts: n_experts.get(),
            },
            Self::UpperBankCandidate { d_model, n_experts } => {
                ModelSizeProfileSerde::UpperBankCandidate {
                    d_model: d_model.get(),
                    n_experts: n_experts.get(),
                }
            }
        };

        raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ModelSizeProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        enum ModelSizeProfileSerde {
            Toy0,
            Toy1,
            #[cfg(feature = "falsify")]
            ToyTiny,
            MoeTiny {
                n_experts: u8,
            },
            UpperBankCandidate {
                d_model: u16,
                n_experts: u8,
            },
        }

        match ModelSizeProfileSerde::deserialize(deserializer)? {
            ModelSizeProfileSerde::Toy0 => Ok(Self::Toy0),
            ModelSizeProfileSerde::Toy1 => Ok(Self::Toy1),
            #[cfg(feature = "falsify")]
            ModelSizeProfileSerde::ToyTiny => Ok(Self::ToyTiny),
            ModelSizeProfileSerde::MoeTiny { n_experts } => {
                Self::moe_tiny(n_experts).map_err(serde::de::Error::custom)
            }
            ModelSizeProfileSerde::UpperBankCandidate { d_model, n_experts } => {
                Self::upper_bank_candidate(d_model, n_experts).map_err(serde::de::Error::custom)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSizeProfileError {
    UnsupportedMoeTinyExperts { n_experts: u8 },
    UnsupportedUpperBankDModel { d_model: u16 },
    UnsupportedUpperBankDff { d_ff: u16 },
    UnsupportedUpperBankExperts { n_experts: u8 },
    ForbiddenDimPair { d_model: u16, d_ff: u16 },
}

impl fmt::Display for ModelSizeProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedMoeTinyExperts { n_experts } => {
                write!(f, "MoeTiny requires n_experts in {{2, 4}}, got {n_experts}")
            }
            Self::UnsupportedUpperBankDModel { d_model } => {
                write!(
                    f,
                    "UpperBankCandidate requires d_model in {{96, 128}}, got {d_model}"
                )
            }
            Self::UnsupportedUpperBankDff { d_ff } => {
                write!(f, "UpperBankCandidate requires d_ff=192, got {d_ff}")
            }
            Self::UnsupportedUpperBankExperts { n_experts } => {
                write!(
                    f,
                    "UpperBankCandidate requires n_experts=4, got {n_experts}"
                )
            }
            Self::ForbiddenDimPair { d_model, d_ff } => {
                write!(f, "model profile dim pair ({d_model}, {d_ff}) is forbidden")
            }
        }
    }
}

impl Error for ModelSizeProfileError {}

fn validate_moe_tiny_experts(n_experts: u8) -> Result<(), ModelSizeProfileError> {
    match n_experts {
        2 | 4 => Ok(()),
        _ => Err(ModelSizeProfileError::UnsupportedMoeTinyExperts { n_experts }),
    }
}

fn validate_upper_bank_dims(d_model: u16, d_ff: u16) -> Result<(), ModelSizeProfileError> {
    if matches!((d_model, d_ff), (128, 256) | (256, 512)) {
        return Err(ModelSizeProfileError::ForbiddenDimPair { d_model, d_ff });
    }

    validate_upper_bank_d_model(d_model)?;
    if d_ff != ModelSizeProfile::UPPER_BANK_D_FF {
        return Err(ModelSizeProfileError::UnsupportedUpperBankDff { d_ff });
    }

    Ok(())
}

fn validate_upper_bank_d_model(d_model: u16) -> Result<(), ModelSizeProfileError> {
    if matches!(d_model, 96 | 128) {
        Ok(())
    } else {
        Err(ModelSizeProfileError::UnsupportedUpperBankDModel { d_model })
    }
}

fn validate_upper_bank_experts(n_experts: u8) -> Result<(), ModelSizeProfileError> {
    if n_experts == ModelSizeProfile::UPPER_BANK_N_EXPERTS {
        Ok(())
    } else {
        Err(ModelSizeProfileError::UnsupportedUpperBankExperts { n_experts })
    }
}

fn default_expert_matrix_byte_cost(rows: u32, cols: u32) -> ByteCost {
    if rows == 0 || cols == 0 {
        return ByteCost::ZERO;
    }

    let weights = ceil_div(u128::from(rows) * u128::from(cols) * 2, 8);
    let scales = u128::from(rows) * 2;
    ByteCost::new(saturating_u64(weights.saturating_add(scales)))
}

fn ceil_div(lhs: u128, rhs: u128) -> u128 {
    lhs / rhs + u128::from(lhs % rhs != 0)
}

fn saturating_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toy0_profile_has_reference_shape_and_no_experts() {
        let profile = ModelSizeProfile::toy0();

        assert_eq!(profile, ModelSizeProfile::Toy0);
        assert_eq!(profile.d_model(), 16);
        assert_eq!(profile.d_ff(), 32);
        assert_eq!(profile.n_blocks(), 1);
        assert_eq!(profile.n_experts(), 0);
        assert_eq!(profile.expert_byte_cost(), ByteCost::new(352));
    }

    #[test]
    fn toy1_profile_has_reference_shape_and_no_experts() {
        let profile = ModelSizeProfile::toy1();

        assert_eq!(profile, ModelSizeProfile::Toy1);
        assert_eq!(profile.d_model(), 32);
        assert_eq!(profile.d_ff(), 64);
        assert_eq!(profile.n_blocks(), 2);
        assert_eq!(profile.n_experts(), 0);
        assert_eq!(profile.expert_byte_cost(), ByteCost::new(1216));
    }

    #[cfg(feature = "falsify")]
    #[test]
    fn toy_tiny_profile_exists_only_for_falsification() {
        let profile = ModelSizeProfile::toy_tiny_for_falsification();

        assert_eq!(profile, ModelSizeProfile::ToyTiny);
        assert_eq!(profile.d_model(), 2);
        assert_eq!(profile.d_ff(), 4);
        assert_eq!(profile.n_blocks(), 1);
        assert_eq!(profile.n_experts(), 0);
    }

    #[test]
    fn moe_tiny_accepts_two_experts() {
        let profile = ModelSizeProfile::moe_tiny(2).expect("two experts is supported");

        assert_eq!(profile.d_model(), 64);
        assert_eq!(profile.d_ff(), 128);
        assert_eq!(profile.n_blocks(), 4);
        assert_eq!(profile.n_experts(), 2);
        assert_eq!(profile.expert_byte_cost(), ByteCost::new(4480));
    }

    #[test]
    fn moe_tiny_accepts_four_experts() {
        let profile = ModelSizeProfile::moe_tiny(4).expect("four experts is supported");

        assert_eq!(profile.n_experts(), 4);
    }

    #[test]
    fn upper_bank_candidate_accepts_ninety_six_width() {
        let profile =
            ModelSizeProfile::upper_bank_candidate(96, 4).expect("96-wide candidate is supported");

        assert_eq!(profile.d_model(), 96);
        assert_eq!(profile.d_ff(), 192);
        assert_eq!(profile.n_blocks(), 4);
        assert_eq!(profile.n_experts(), 4);
        assert_eq!(profile.expert_byte_cost(), ByteCost::new(9792));
    }

    #[test]
    fn upper_bank_candidate_accepts_one_twenty_eight_width_with_192_ff() {
        let profile = ModelSizeProfile::upper_bank_candidate(128, 4).expect("128x192 is supported");

        assert_eq!(profile.d_model(), 128);
        assert_eq!(profile.d_ff(), 192);
        assert_eq!(profile.n_blocks(), 4);
        assert_eq!(profile.n_experts(), 4);
        assert_eq!(profile.expert_byte_cost(), ByteCost::new(12928));
    }

    #[test]
    fn moe_tiny_rejects_invalid_expert_counts() {
        for n_experts in [0, 1, 3, 5] {
            assert_eq!(
                ModelSizeProfile::moe_tiny(n_experts),
                Err(ModelSizeProfileError::UnsupportedMoeTinyExperts { n_experts })
            );
        }
    }

    #[test]
    fn upper_bank_candidate_rejects_invalid_experts_and_dims() {
        assert_eq!(
            ModelSizeProfile::upper_bank_candidate(96, 2),
            Err(ModelSizeProfileError::UnsupportedUpperBankExperts { n_experts: 2 })
        );
        assert_eq!(
            ModelSizeProfile::upper_bank_candidate(64, 4),
            Err(ModelSizeProfileError::UnsupportedUpperBankDModel { d_model: 64 })
        );
        assert_eq!(
            ModelSizeProfile::upper_bank_candidate_with_dims(96, 128, 4),
            Err(ModelSizeProfileError::UnsupportedUpperBankDff { d_ff: 128 })
        );
    }

    #[test]
    fn upper_bank_candidate_rejects_forbidden_dim_caps_before_other_shape_checks() {
        assert_eq!(
            ModelSizeProfile::upper_bank_candidate_with_dims(128, 256, 4),
            Err(ModelSizeProfileError::ForbiddenDimPair {
                d_model: 128,
                d_ff: 256
            })
        );
        assert_eq!(
            ModelSizeProfile::upper_bank_candidate_with_dims(256, 512, 4),
            Err(ModelSizeProfileError::ForbiddenDimPair {
                d_model: 256,
                d_ff: 512
            })
        );
    }

    #[test]
    fn serde_deserializes_only_through_validated_constructors() {
        let valid = serde_json::json!({"UpperBankCandidate": {"d_model": 96, "n_experts": 4}});
        let decoded: ModelSizeProfile =
            serde_json::from_value(valid).expect("valid upper-bank profile deserializes");
        assert_eq!(decoded.d_model(), 96);
        assert_eq!(decoded.n_experts(), 4);

        let invalid = serde_json::json!({"UpperBankCandidate": {"d_model": 128, "n_experts": 2}});
        let err = serde_json::from_value::<ModelSizeProfile>(invalid)
            .expect_err("invalid upper-bank profile is rejected by deserialization");
        assert!(err.to_string().contains("n_experts=4"));

        let invalid = serde_json::json!({"MoeTiny": {"n_experts": 3}});
        let err = serde_json::from_value::<ModelSizeProfile>(invalid)
            .expect_err("invalid MoeTiny profile is rejected by deserialization");
        assert!(err.to_string().contains("n_experts in {2, 4}"));
    }
}
