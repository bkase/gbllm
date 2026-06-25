//! WRAM layout policy types for compile profiles and runtime budgets.

use std::error::Error;
use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};

pub const DMG_WRAM_SIZE_BYTES: u32 = gbf_hw::memory::WRAM_SIZE_BYTES;

/// Fixed WRAM split requested by a compile profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WramLayoutPolicy {
    /// Fixed-size region reserved for overlay payloads; constant per `CompileProfile`.
    pub overlay_bytes: u16,
    /// WRAM bytes reserved for the resumable continuation record.
    pub continuation_bytes: u16,
    /// WRAM bytes reserved for the call stack.
    pub stack_bytes: u16,
    /// Floor the hot ArenaPlan must satisfy; actual allocation may exceed it.
    pub hot_arena_bytes_min: u16,
}

impl WramLayoutPolicy {
    pub fn new(
        overlay_bytes: u16,
        continuation_bytes: u16,
        stack_bytes: u16,
        hot_arena_bytes_min: u16,
    ) -> Result<Self, WramPolicyError> {
        let policy = Self {
            overlay_bytes,
            continuation_bytes,
            stack_bytes,
            hot_arena_bytes_min,
        };
        let required_bytes = policy.required_wram_bytes();
        if required_bytes > DMG_WRAM_SIZE_BYTES {
            return Err(WramPolicyError::LayoutExceedsWram {
                required_bytes,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            });
        }
        debug_assert!(required_bytes <= DMG_WRAM_SIZE_BYTES);
        Ok(policy)
    }

    #[must_use]
    pub const fn bringup_defaults() -> Self {
        Self {
            overlay_bytes: 512,
            continuation_bytes: 256,
            stack_bytes: 256,
            hot_arena_bytes_min: 4096,
        }
    }

    #[must_use]
    pub const fn runtime_internal_bytes(self) -> u32 {
        self.continuation_bytes as u32 + self.stack_bytes as u32
    }

    #[must_use]
    pub const fn required_wram_bytes(self) -> u32 {
        self.overlay_bytes as u32
            + self.continuation_bytes as u32
            + self.stack_bytes as u32
            + self.hot_arena_bytes_min as u32
    }
}

impl<'de> Deserialize<'de> for WramLayoutPolicy {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            overlay_bytes: u16,
            continuation_bytes: u16,
            stack_bytes: u16,
            hot_arena_bytes_min: u16,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(
            repr.overlay_bytes,
            repr.continuation_bytes,
            repr.stack_bytes,
            repr.hot_arena_bytes_min,
        )
        .map_err(D::Error::custom)
    }
}

/// Overlay reload cadence named by a compile profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
#[allow(
    clippy::enum_variant_names,
    reason = "T11.1/RFC names all reload-cadence variants with the Per prefix"
)]
pub enum OverlayReloadPolicy {
    /// Reload only when the routed expert or kernel family changes.
    #[default]
    PerExpertSwitch,
    /// Reload at every safe-point/yield.
    PerSlice,
    /// Reload at each layer's expert dispatch.
    PerLayerStep,
}

/// WRAM reservation summary emitted by a runtime budget.
///
/// `total` is always validated to be at most `DMG_WRAM_SIZE_BYTES` (8 KiB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WramReserved {
    pub overlay: u16,
    pub hot_arena_floor: u16,
    pub total: u16,
}

impl WramReserved {
    pub fn new(overlay: u16, hot_arena_floor: u16, total: u16) -> Result<Self, WramPolicyError> {
        if u32::from(total) > DMG_WRAM_SIZE_BYTES {
            return Err(WramPolicyError::ReservedTotalExceedsWram {
                total,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            });
        }
        if overlay > total {
            return Err(WramPolicyError::ReservedOverlayExceedsTotal { overlay, total });
        }
        if u32::from(hot_arena_floor) > DMG_WRAM_SIZE_BYTES {
            return Err(WramPolicyError::HotArenaFloorExceedsWram {
                hot_arena_floor,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            });
        }

        let reserved = Self {
            overlay,
            hot_arena_floor,
            total,
        };
        debug_assert!(u32::from(reserved.total) <= DMG_WRAM_SIZE_BYTES);
        Ok(reserved)
    }

    #[must_use]
    pub const fn runtime_internal_bytes(self) -> u16 {
        self.total.saturating_sub(self.overlay)
    }
}

impl<'de> Deserialize<'de> for WramReserved {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            overlay: u16,
            hot_arena_floor: u16,
            total: u16,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(repr.overlay, repr.hot_arena_floor, repr.total).map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WramPolicyError {
    LayoutExceedsWram {
        required_bytes: u32,
        wram_bytes: u32,
    },
    ReservedTotalExceedsWram {
        total: u16,
        wram_bytes: u32,
    },
    ReservedOverlayExceedsTotal {
        overlay: u16,
        total: u16,
    },
    HotArenaFloorExceedsWram {
        hot_arena_floor: u16,
        wram_bytes: u32,
    },
}

impl fmt::Display for WramPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LayoutExceedsWram {
                required_bytes,
                wram_bytes,
            } => write!(
                f,
                "WRAM layout requires {required_bytes} bytes, exceeding {wram_bytes} byte WRAM"
            ),
            Self::ReservedTotalExceedsWram { total, wram_bytes } => write!(
                f,
                "WRAM reserved total {total} exceeds {wram_bytes} byte WRAM"
            ),
            Self::ReservedOverlayExceedsTotal { overlay, total } => {
                write!(f, "WRAM overlay {overlay} exceeds reserved total {total}")
            }
            Self::HotArenaFloorExceedsWram {
                hot_arena_floor,
                wram_bytes,
            } => write!(
                f,
                "WRAM hot_arena_floor {hot_arena_floor} exceeds {wram_bytes} byte WRAM"
            ),
        }
    }
}

impl Error for WramPolicyError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wram_layout_policy_round_trips_and_pins_json_shape() {
        let policy = WramLayoutPolicy::new(4096, 256, 256, 2048).expect("valid layout");
        let expected = serde_json::json!({
            "overlay_bytes": 4096,
            "continuation_bytes": 256,
            "stack_bytes": 256,
            "hot_arena_bytes_min": 2048
        });

        assert_eq!(policy.required_wram_bytes(), 6656);
        assert_eq!(policy.runtime_internal_bytes(), 512);
        assert_eq!(
            serde_json::to_value(policy).expect("layout serializes"),
            expected
        );
        assert_eq!(
            serde_json::from_value::<WramLayoutPolicy>(expected).expect("layout deserializes"),
            policy
        );
    }

    #[test]
    fn wram_layout_bringup_defaults_match_compile_profile() {
        let policy = WramLayoutPolicy::bringup_defaults();

        assert_eq!(policy.overlay_bytes, 512);
        assert_eq!(policy.continuation_bytes, 256);
        assert_eq!(policy.stack_bytes, 256);
        assert_eq!(policy.hot_arena_bytes_min, 4096);
        assert_eq!(policy.required_wram_bytes(), 5120);
        assert!(policy.required_wram_bytes() <= DMG_WRAM_SIZE_BYTES);
    }

    #[test]
    fn wram_layout_policy_rejects_over_wram_construction_and_deserialize() {
        let error = WramLayoutPolicy::new(8192, 1, 0, 0).expect_err("layout exceeds WRAM");
        assert_eq!(
            error,
            WramPolicyError::LayoutExceedsWram {
                required_bytes: 8193,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            }
        );

        let value = serde_json::json!({
            "overlay_bytes": 8192,
            "continuation_bytes": 1,
            "stack_bytes": 0,
            "hot_arena_bytes_min": 0
        });
        assert!(serde_json::from_value::<WramLayoutPolicy>(value).is_err());
    }

    #[test]
    fn wram_layout_policy_rejects_unknown_fields() {
        let value = serde_json::json!({
            "overlay_bytes": 4096,
            "continuation_bytes": 256,
            "stack_bytes": 256,
            "hot_arena_bytes_min": 2048,
            "surprise": true
        });

        assert!(serde_json::from_value::<WramLayoutPolicy>(value).is_err());
    }

    #[test]
    fn wram_layout_overlay_reload_policy_shape_and_default_are_pinned() {
        assert_eq!(
            OverlayReloadPolicy::default(),
            OverlayReloadPolicy::PerExpertSwitch
        );
        assert_eq!(
            serde_json::to_value(OverlayReloadPolicy::PerExpertSwitch).expect("policy serializes"),
            serde_json::json!({ "kind": "PerExpertSwitch" })
        );
        assert_eq!(
            serde_json::to_value(OverlayReloadPolicy::PerSlice).expect("policy serializes"),
            serde_json::json!({ "kind": "PerSlice" })
        );
        assert_eq!(
            serde_json::to_value(OverlayReloadPolicy::PerLayerStep).expect("policy serializes"),
            serde_json::json!({ "kind": "PerLayerStep" })
        );
    }

    #[test]
    fn wram_layout_reserved_round_trips_and_pins_json_shape() {
        let reserved = WramReserved::new(4096, 2048, 8192).expect("valid reservation");
        let expected = serde_json::json!({
            "overlay": 4096,
            "hot_arena_floor": 2048,
            "total": 8192
        });

        assert_eq!(reserved.runtime_internal_bytes(), 4096);
        assert_eq!(
            serde_json::to_value(reserved).expect("reservation serializes"),
            expected
        );
        assert_eq!(
            serde_json::from_value::<WramReserved>(expected).expect("reservation deserializes"),
            reserved
        );
    }

    #[test]
    fn wram_layout_reserved_rejects_invalid_bounds() {
        assert_eq!(
            WramReserved::new(4096, 0, 4095).expect_err("overlay exceeds total"),
            WramPolicyError::ReservedOverlayExceedsTotal {
                overlay: 4096,
                total: 4095,
            }
        );
        assert_eq!(
            WramReserved::new(0, 0, 8193).expect_err("total exceeds WRAM"),
            WramPolicyError::ReservedTotalExceedsWram {
                total: 8193,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            }
        );
        assert_eq!(
            WramReserved::new(0, 8193, 0).expect_err("hot arena exceeds WRAM"),
            WramPolicyError::HotArenaFloorExceedsWram {
                hot_arena_floor: 8193,
                wram_bytes: DMG_WRAM_SIZE_BYTES,
            }
        );

        let value = serde_json::json!({
            "overlay": 4096,
            "hot_arena_floor": 0,
            "total": 4095
        });
        assert!(serde_json::from_value::<WramReserved>(value).is_err());
    }
}
