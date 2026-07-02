//! Runtime chrome budget schema.

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use gbf_abi::RuntimeShellModule;
use gbf_foundation::{BudgetSlotId, CompileProfileId, Hash256, TargetProfileId};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::canonical::domain_hash;
use crate::compile::PlacementProfile;
use crate::reference_shell::pinned_reference_shell;
use crate::wram::WramReserved;

pub const SYNTHETIC_REFERENCE_PREFIX: &str = "SYNTHETIC_REFERENCE:";

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RuntimeNucleusHash {
    Real(Hash256),
    SyntheticReference(Hash256),
}

impl RuntimeNucleusHash {
    #[must_use]
    pub const fn real(hash: Hash256) -> Self {
        Self::Real(hash)
    }

    #[must_use]
    pub const fn synthetic_reference(hash: Hash256) -> Self {
        Self::SyntheticReference(hash)
    }

    #[must_use]
    pub const fn hash(self) -> Hash256 {
        match self {
            Self::Real(hash) | Self::SyntheticReference(hash) => hash,
        }
    }

    #[must_use]
    pub const fn is_synthetic_reference(self) -> bool {
        matches!(self, Self::SyntheticReference(_))
    }
}

impl From<Hash256> for RuntimeNucleusHash {
    fn from(hash: Hash256) -> Self {
        Self::real(hash)
    }
}

impl fmt::Display for RuntimeNucleusHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Real(hash) => fmt::Display::fmt(hash, f),
            Self::SyntheticReference(hash) => {
                f.write_str(SYNTHETIC_REFERENCE_PREFIX)?;
                fmt::Display::fmt(hash, f)
            }
        }
    }
}

impl fmt::Debug for RuntimeNucleusHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for RuntimeNucleusHash {
    type Err = RuntimeNucleusHashParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(hash) = value.strip_prefix(SYNTHETIC_REFERENCE_PREFIX) {
            return Hash256::from_str(hash)
                .map(Self::synthetic_reference)
                .map_err(RuntimeNucleusHashParseError::Hash);
        }

        Hash256::from_str(value)
            .map(Self::real)
            .map_err(RuntimeNucleusHashParseError::Hash)
    }
}

impl Serialize for RuntimeNucleusHash {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for RuntimeNucleusHash {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeNucleusHashParseError {
    Hash(gbf_foundation::Hash256ParseError),
}

impl fmt::Display for RuntimeNucleusHashParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hash(error) => fmt::Display::fmt(error, f),
        }
    }
}

impl std::error::Error for RuntimeNucleusHashParseError {}

pub const RUNTIME_NUCLEUS_MODULE_BINDING_SCHEMA_VERSION: &str = "runtime_nucleus_modules.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeChromeBudget {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: RuntimeNucleusHash,
    pub reference_shell_modules: BTreeSet<RuntimeShellModule>,
    pub rom_slots: Vec<RomBudgetSlot>,
    pub memory_caps: RuntimeMemoryCapSection,
    pub wram_reserved: WramReserved,
    pub sram_reserved: u32,
}

impl RuntimeChromeBudget {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        target: TargetProfileId,
        profile: CompileProfileId,
        runtime_nucleus_hash: RuntimeNucleusHash,
        reference_shell_modules: BTreeSet<RuntimeShellModule>,
        rom_slots: Vec<RomBudgetSlot>,
        memory_caps: RuntimeMemoryCapSection,
        wram_reserved: WramReserved,
        sram_reserved: u32,
    ) -> Result<Self, RuntimeChromeBudgetValidationError> {
        let budget = Self {
            target,
            profile,
            runtime_nucleus_hash,
            reference_shell_modules,
            rom_slots,
            memory_caps,
            wram_reserved,
            sram_reserved,
        };
        budget.validate()?;
        Ok(budget)
    }

    pub fn validate(&self) -> Result<(), RuntimeChromeBudgetValidationError> {
        if self.rom_slots.is_empty() {
            return Err(RuntimeChromeBudgetValidationError::NoRomSlots);
        }

        let mut seen_slots = BTreeSet::new();
        for slot in &self.rom_slots {
            slot.validate()?;
            if !seen_slots.insert(slot.id) {
                return Err(RuntimeChromeBudgetValidationError::DuplicateSlotId { id: slot.id });
            }
        }

        if self.reference_shell_modules.is_empty() {
            return Err(RuntimeChromeBudgetValidationError::NoReferenceShellModules);
        }

        if u32::from(self.wram_reserved.total) > self.memory_caps.wram_usable_bytes {
            return Err(RuntimeChromeBudgetValidationError::WramReservedOutOfRange {
                wram_reserved: self.wram_reserved,
                wram_usable_bytes: self.memory_caps.wram_usable_bytes,
            });
        }

        if u32::from(self.wram_reserved.hot_arena_floor) > self.memory_caps.wram_usable_bytes {
            return Err(
                RuntimeChromeBudgetValidationError::WramHotArenaFloorOutOfRange {
                    hot_arena_floor: self.wram_reserved.hot_arena_floor,
                    wram_usable_bytes: self.memory_caps.wram_usable_bytes,
                },
            );
        }

        if self.sram_reserved > self.memory_caps.sram_usable_bytes {
            return Err(RuntimeChromeBudgetValidationError::SramReservedOutOfRange {
                sram_reserved: self.sram_reserved,
                sram_usable_bytes: self.memory_caps.sram_usable_bytes,
            });
        }

        Ok(())
    }

    /// Hash the runtime image hash together with the module set that emitted it.
    ///
    /// T2.1 does not implement the runtime emitter. This helper gives bd-1g9 /
    /// bd-177 a stable policy-side binding so adding or removing a shell module
    /// changes the hash preimage even when byte sizes happen to match.
    pub fn runtime_nucleus_module_binding_hash(
        runtime_image_hash: Hash256,
        reference_shell_modules: &BTreeSet<RuntimeShellModule>,
    ) -> Result<Hash256, serde_json::Error> {
        #[derive(Serialize)]
        struct Binding<'a> {
            runtime_image_hash: Hash256,
            reference_shell_modules: &'a BTreeSet<RuntimeShellModule>,
        }

        domain_hash(
            "gbf-policy",
            "RuntimeChromeBudget",
            RUNTIME_NUCLEUS_MODULE_BINDING_SCHEMA_VERSION,
            &Binding {
                runtime_image_hash,
                reference_shell_modules,
            },
        )
    }

    #[must_use]
    pub fn pinned_reference_shell_modules() -> BTreeSet<RuntimeShellModule> {
        pinned_reference_shell().included
    }

    #[must_use]
    pub const fn wram_reserved_total_bytes(&self) -> u16 {
        self.wram_reserved.total
    }

    #[must_use]
    pub const fn wram_overlay_bytes(&self) -> u16 {
        self.wram_reserved.overlay
    }
}

impl<'de> Deserialize<'de> for RuntimeChromeBudget {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            target: TargetProfileId,
            profile: CompileProfileId,
            runtime_nucleus_hash: RuntimeNucleusHash,
            reference_shell_modules: BTreeSet<RuntimeShellModule>,
            rom_slots: Vec<RomBudgetSlot>,
            memory_caps: RuntimeMemoryCapSection,
            wram_reserved: WramReserved,
            sram_reserved: u32,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(
            repr.target,
            repr.profile,
            repr.runtime_nucleus_hash,
            repr.reference_shell_modules,
            repr.rom_slots,
            repr.memory_caps,
            repr.wram_reserved,
            repr.sram_reserved,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeMemoryCapSection {
    pub wram_usable_bytes: u32,
    pub sram_usable_bytes: u32,
    pub hram_usable_bytes: u32,
    pub source_target_profile_hash: Hash256,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RomBudgetSlot {
    pub id: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    /// Named slack covers bank headers/stubs plus future reservations from
    /// `ReferenceShellSpec`; it is not an opaque safety fudge factor.
    pub reserved_slack: u16,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

impl RomBudgetSlot {
    pub fn new(
        id: BudgetSlotId,
        class: BudgetSlotClass,
        usable_bytes: u32,
        reserved_slack: u16,
        placement_caps: BTreeSet<PlacementProfile>,
    ) -> Result<Self, RuntimeChromeBudgetValidationError> {
        let slot = Self {
            id,
            class,
            usable_bytes,
            reserved_slack,
            placement_caps,
        };
        slot.validate()?;
        Ok(slot)
    }

    pub fn validate(&self) -> Result<(), RuntimeChromeBudgetValidationError> {
        if self.usable_bytes == 0 {
            return Err(RuntimeChromeBudgetValidationError::ZeroUsableBytes { id: self.id });
        }
        if u32::from(self.reserved_slack) >= self.usable_bytes {
            return Err(
                RuntimeChromeBudgetValidationError::ReservedSlackOutOfRange {
                    id: self.id,
                    reserved_slack: self.reserved_slack,
                    usable_bytes: self.usable_bytes,
                },
            );
        }
        if self.placement_caps.is_empty() {
            return Err(RuntimeChromeBudgetValidationError::EmptyPlacementCaps { id: self.id });
        }
        Ok(())
    }
}

impl<'de> Deserialize<'de> for RomBudgetSlot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Repr {
            id: BudgetSlotId,
            class: BudgetSlotClass,
            usable_bytes: u32,
            reserved_slack: u16,
            placement_caps: BTreeSet<PlacementProfile>,
        }

        let repr = Repr::deserialize(deserializer)?;
        Self::new(
            repr.id,
            repr.class,
            repr.usable_bytes,
            repr.reserved_slack,
            repr.placement_caps,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum BudgetSlotClass {
    Bank0Free,
    CommonBank,
    ExpertBank,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeChromeBudgetValidationError {
    NoRomSlots,
    NoReferenceShellModules,
    DuplicateSlotId {
        id: BudgetSlotId,
    },
    ZeroUsableBytes {
        id: BudgetSlotId,
    },
    ReservedSlackOutOfRange {
        id: BudgetSlotId,
        reserved_slack: u16,
        usable_bytes: u32,
    },
    EmptyPlacementCaps {
        id: BudgetSlotId,
    },
    WramReservedOutOfRange {
        wram_reserved: WramReserved,
        wram_usable_bytes: u32,
    },
    WramHotArenaFloorOutOfRange {
        hot_arena_floor: u16,
        wram_usable_bytes: u32,
    },
    SramReservedOutOfRange {
        sram_reserved: u32,
        sram_usable_bytes: u32,
    },
}

impl fmt::Display for RuntimeChromeBudgetValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoRomSlots => write!(f, "RuntimeChromeBudget must contain at least one ROM slot"),
            Self::NoReferenceShellModules => write!(
                f,
                "RuntimeChromeBudget must record at least one reference shell module"
            ),
            Self::DuplicateSlotId { id } => write!(f, "duplicate ROM budget slot id {id}"),
            Self::ZeroUsableBytes { id } => {
                write!(f, "ROM budget slot {id} must have nonzero usable_bytes")
            }
            Self::ReservedSlackOutOfRange {
                id,
                reserved_slack,
                usable_bytes,
            } => write!(
                f,
                "ROM budget slot {id} reserved_slack {reserved_slack} must be below usable_bytes {usable_bytes}"
            ),
            Self::EmptyPlacementCaps { id } => {
                write!(
                    f,
                    "ROM budget slot {id} must include at least one placement cap"
                )
            }
            Self::WramReservedOutOfRange {
                wram_reserved,
                wram_usable_bytes,
            } => write!(
                f,
                "wram_reserved.total {} must be at or below wram_usable_bytes {wram_usable_bytes}",
                wram_reserved.total
            ),
            Self::WramHotArenaFloorOutOfRange {
                hot_arena_floor,
                wram_usable_bytes,
            } => write!(
                f,
                "wram_reserved.hot_arena_floor {hot_arena_floor} must be at or below wram_usable_bytes {wram_usable_bytes}"
            ),
            Self::SramReservedOutOfRange {
                sram_reserved,
                sram_usable_bytes,
            } => write!(
                f,
                "sram_reserved {sram_reserved} must not exceed sram_usable_bytes {sram_usable_bytes}"
            ),
        }
    }
}

impl std::error::Error for RuntimeChromeBudgetValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_json(byte: u8) -> serde_json::Value {
        serde_json::to_value(Hash256::from_bytes([byte; 32])).expect("hash serializes")
    }

    fn runtime_hash(byte: u8) -> RuntimeNucleusHash {
        RuntimeNucleusHash::real(Hash256::from_bytes([byte; 32]))
    }

    fn default_wram_reserved() -> WramReserved {
        WramReserved::new(128, 4096, 128).expect("valid WRAM reservation")
    }

    fn budget_fixture() -> RuntimeChromeBudget {
        RuntimeChromeBudget::new(
            TargetProfileId::from("dmg-mbc5"),
            CompileProfileId::from("Bringup"),
            runtime_hash(1),
            RuntimeChromeBudget::pinned_reference_shell_modules(),
            vec![
                RomBudgetSlot::new(
                    BudgetSlotId::new(7),
                    BudgetSlotClass::ExpertBank,
                    16_000,
                    384,
                    BTreeSet::from([
                        PlacementProfile::StrictOnePerBank,
                        PlacementProfile::Budgeted,
                    ]),
                )
                .expect("valid slot"),
            ],
            RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: 32 * 1024,
                hram_usable_bytes: 127,
                source_target_profile_hash: Hash256::from_bytes([2; 32]),
            },
            default_wram_reserved(),
            512,
        )
        .expect("valid budget")
    }

    #[test]
    fn budget_types_round_trip() {
        let budget = budget_fixture();
        let expected = serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "runtime_nucleus_hash": hash_json(1),
            "reference_shell_modules": [
                "boot",
                "interrupts",
                "scheduler",
                "banking",
                "joypad",
                "text",
                "keyboard",
                "video_commit"
            ],
            "rom_slots": [
                {
                    "id": 7,
                    "class": {"kind": "ExpertBank"},
                    "usable_bytes": 16000,
                    "reserved_slack": 384,
                    "placement_caps": [
                        {"kind": "StrictOnePerBank"},
                        {"kind": "Budgeted"}
                    ]
                }
            ],
            "memory_caps": {
                "wram_usable_bytes": 8192,
                "sram_usable_bytes": 32768,
                "hram_usable_bytes": 127,
                "source_target_profile_hash": hash_json(2)
            },
            "wram_reserved": {
                "overlay": 128,
                "hot_arena_floor": 4096,
                "total": 128
            },
            "sram_reserved": 512
        });

        let encoded = serde_json::to_string(&budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_str(&encoded).expect("budget deserializes");

        assert_eq!(decoded, budget);
        assert_eq!(
            serde_json::to_value(&budget).expect("budget serializes"),
            expected
        );
    }

    #[test]
    fn memory_cap_section_preserves_u32_json_widths() {
        let memory_caps = RuntimeMemoryCapSection {
            wram_usable_bytes: 70_000,
            sram_usable_bytes: u32::MAX,
            hram_usable_bytes: 300,
            source_target_profile_hash: Hash256::from_bytes([3; 32]),
        };
        let expected = serde_json::json!({
            "wram_usable_bytes": 70000,
            "sram_usable_bytes": 4294967295u64,
            "hram_usable_bytes": 300,
            "source_target_profile_hash": hash_json(3)
        });

        let encoded = serde_json::to_value(memory_caps).expect("memory caps serialize");
        let decoded: RuntimeMemoryCapSection =
            serde_json::from_value(expected.clone()).expect("memory caps deserialize");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, memory_caps);
    }

    #[test]
    fn runtime_chrome_budget_preserves_valid_reserved_field_boundaries() {
        let budget = RuntimeChromeBudget::new(
            TargetProfileId::from("dmg-mbc5"),
            CompileProfileId::from("Bringup"),
            runtime_hash(1),
            RuntimeChromeBudget::pinned_reference_shell_modules(),
            budget_fixture().rom_slots,
            RuntimeMemoryCapSection {
                wram_usable_bytes: 8 * 1024,
                sram_usable_bytes: u32::MAX,
                hram_usable_bytes: 127,
                source_target_profile_hash: Hash256::from_bytes([2; 32]),
            },
            WramReserved::new(8 * 1024, 8 * 1024, 8 * 1024).expect("boundary WRAM reservation"),
            u32::MAX,
        )
        .expect("boundary budget remains valid");
        let expected = serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "runtime_nucleus_hash": hash_json(1),
            "reference_shell_modules": [
                "boot",
                "interrupts",
                "scheduler",
                "banking",
                "joypad",
                "text",
                "keyboard",
                "video_commit"
            ],
            "rom_slots": [
                {
                    "id": 7,
                    "class": {"kind": "ExpertBank"},
                    "usable_bytes": 16000,
                    "reserved_slack": 384,
                    "placement_caps": [
                        {"kind": "StrictOnePerBank"},
                        {"kind": "Budgeted"}
                    ]
                }
            ],
            "memory_caps": {
                "wram_usable_bytes": 8192,
                "sram_usable_bytes": 4294967295u64,
                "hram_usable_bytes": 127,
                "source_target_profile_hash": hash_json(2)
            },
            "wram_reserved": {
                "overlay": 8192,
                "hot_arena_floor": 8192,
                "total": 8192
            },
            "sram_reserved": u32::MAX
        });

        let encoded = serde_json::to_value(&budget).expect("budget serializes");
        let decoded: RuntimeChromeBudget =
            serde_json::from_value(expected.clone()).expect("budget deserializes");

        assert_eq!(encoded, expected);
        assert_eq!(decoded, budget);
    }

    #[test]
    fn runtime_chrome_budget_deserialize_rejects_invalid_reserved_counts() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["wram_reserved"] = serde_json::json!({
            "overlay": 8192,
            "hot_arena_floor": 4096,
            "total": 8193
        });

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("invalid wram reservation is rejected");

        assert!(error.to_string().contains("WRAM reserved total"));

        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["sram_reserved"] = serde_json::json!(32769);

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("invalid sram reservation is rejected");

        assert!(error.to_string().contains("sram_reserved"));
    }

    #[test]
    fn runtime_chrome_budget_deserialize_rejects_empty_reference_shell_modules() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["reference_shell_modules"] = serde_json::json!([]);

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("empty module set is rejected");

        assert!(error.to_string().contains("reference shell module"));
    }

    #[test]
    fn runtime_chrome_budget_deserialize_rejects_bad_slots() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["rom_slots"][0]["reserved_slack"] = serde_json::json!(16_000);

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("slot slack must fit below usable bytes");

        assert!(error.to_string().contains("reserved_slack"));

        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["rom_slots"][0]["placement_caps"] = serde_json::json!([]);

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("placement caps must be nonempty");

        assert!(error.to_string().contains("placement cap"));
    }

    #[test]
    fn runtime_chrome_budget_deserialize_rejects_duplicate_slots() {
        let slot = serde_json::json!({
            "id": 7,
            "class": {"kind": "ExpertBank"},
            "usable_bytes": 16000,
            "reserved_slack": 384,
            "placement_caps": [{"kind": "Budgeted"}]
        });
        let value = serde_json::json!({
            "target": "dmg-mbc5",
            "profile": "Bringup",
            "runtime_nucleus_hash": hash_json(1),
            "reference_shell_modules": [
                "boot",
                "interrupts",
                "scheduler",
                "banking",
                "joypad",
                "text",
                "keyboard",
                "video_commit"
            ],
            "rom_slots": [slot, slot],
            "memory_caps": {
                "wram_usable_bytes": 8192,
                "sram_usable_bytes": 32768,
                "hram_usable_bytes": 127,
                "source_target_profile_hash": hash_json(2)
            },
            "wram_reserved": {
                "overlay": 128,
                "hot_arena_floor": 4096,
                "total": 128
            },
            "sram_reserved": 512
        });

        let error = serde_json::from_value::<RuntimeChromeBudget>(value)
            .expect_err("duplicate slot ids are rejected");

        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn budget_rejects_unknown_field() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["unexpected"] = serde_json::json!("nope");

        assert!(serde_json::from_value::<RuntimeChromeBudget>(value).is_err());
    }

    #[test]
    fn runtime_nucleus_hash_round_trips_real_and_synthetic() {
        let real = RuntimeNucleusHash::real(Hash256::from_bytes([0xab; 32]));
        let synthetic = RuntimeNucleusHash::synthetic_reference(Hash256::from_bytes([0xcd; 32]));

        assert_eq!(
            serde_json::to_value(real).expect("real hash serializes"),
            serde_json::json!(
                "sha256:abababababababababababababababababababababababababababababababab"
            )
        );
        assert_eq!(
            serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(
                "sha256:abababababababababababababababababababababababababababababababab"
            ))
            .expect("real hash deserializes"),
            real
        );
        assert_eq!(
            serde_json::to_value(synthetic).expect("synthetic hash serializes"),
            serde_json::json!(
                "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
            )
        );
        assert_eq!(
            serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(
                "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
            ))
            .expect("synthetic hash deserializes"),
            synthetic
        );
        assert!(synthetic.is_synthetic_reference());
        assert_eq!(
            format!("{synthetic:?}"),
            "SYNTHETIC_REFERENCE:sha256:cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd"
        );
    }

    #[test]
    fn runtime_nucleus_hash_rejects_malformed_strings() {
        for value in [
            "SYNTHETIC_REFERENCE:foo",
            "foo",
            "SYNTHETIC_REFERENCE:SYNTHETIC_REFERENCE:sha256:abababababababababababababababababababababababababababababababab",
            "SYNTHETIC_REFERENCE:sha256:ABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABABAB",
        ] {
            assert!(
                serde_json::from_value::<RuntimeNucleusHash>(serde_json::json!(value)).is_err(),
                "{value} must be rejected"
            );
        }
    }

    #[test]
    fn runtime_chrome_budget_accepts_synthetic_reference_hash() {
        let mut value = serde_json::to_value(budget_fixture()).expect("budget serializes");
        value["runtime_nucleus_hash"] = serde_json::json!(
            "SYNTHETIC_REFERENCE:sha256:0101010101010101010101010101010101010101010101010101010101010101"
        );

        let decoded: RuntimeChromeBudget =
            serde_json::from_value(value).expect("synthetic budget deserializes");

        assert_eq!(
            decoded.runtime_nucleus_hash,
            RuntimeNucleusHash::synthetic_reference(Hash256::from_bytes([1; 32]))
        );
    }

    #[test]
    fn runtime_nucleus_module_binding_hash_changes_when_modules_change() {
        let runtime_image_hash = Hash256::from_bytes([0x77; 32]);
        let modules = RuntimeChromeBudget::pinned_reference_shell_modules();
        assert_eq!(
            modules,
            BTreeSet::from([
                RuntimeShellModule::Boot,
                RuntimeShellModule::Interrupts,
                RuntimeShellModule::Scheduler,
                RuntimeShellModule::Banking,
                RuntimeShellModule::Joypad,
                RuntimeShellModule::Text,
                RuntimeShellModule::Keyboard,
                RuntimeShellModule::VideoCommit,
            ])
        );

        let base =
            RuntimeChromeBudget::runtime_nucleus_module_binding_hash(runtime_image_hash, &modules)
                .expect("binding hash computes");
        let repeated =
            RuntimeChromeBudget::runtime_nucleus_module_binding_hash(runtime_image_hash, &modules)
                .expect("binding hash recomputes");

        let mut expanded = modules;
        expanded.insert(RuntimeShellModule::Panic);
        let changed =
            RuntimeChromeBudget::runtime_nucleus_module_binding_hash(runtime_image_hash, &expanded)
                .expect("expanded binding hash computes");

        assert_eq!(base, repeated);
        assert_ne!(base, changed);
    }

    #[test]
    fn budget_slot_class_json_shapes_are_pinned() {
        assert_eq!(
            serde_json::to_value(BudgetSlotClass::Bank0Free).expect("class serializes"),
            serde_json::json!({ "kind": "Bank0Free" })
        );
        assert_eq!(
            serde_json::to_value(BudgetSlotClass::CommonBank).expect("class serializes"),
            serde_json::json!({ "kind": "CommonBank" })
        );
        assert_eq!(
            serde_json::to_value(BudgetSlotClass::ExpertBank).expect("class serializes"),
            serde_json::json!({ "kind": "ExpertBank" })
        );
    }
}
