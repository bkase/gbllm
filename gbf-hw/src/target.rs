//! Target, console, cartridge, and capability contracts.

use std::fmt;

use gbf_foundation::{Hash256, TargetFamilyId, TargetProfileId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cartridge_header::{MbcType, RamSize, RomSize};
use crate::timing::{TimingProfile, dmg_timing};

pub const BRINGUP_TARGET_PROFILE_ID: &str = "dmg-mbc5-8mib-128kib";
pub const DMG_TARGET_FAMILY_ID: &str = "dmg";

/// RFC-shaped domain separator for canonical `TargetProfile` content hashes.
///
/// The preimage is this byte string followed by canonical JSON for the full
/// public `TargetProfile` representation. The NUL suffix prevents accidental
/// concatenation ambiguity with the first JSON byte.
pub const TARGET_PROFILE_CONTENT_HASH_DOMAIN: &[u8] =
    b"gbf:gbf-hw:TargetProfile:content_hash:1.0.0\0";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "TargetProfileRepr")]
pub struct TargetProfile {
    id: TargetProfileId,
    family: TargetFamilyId,
    console: ConsoleModel,
    cartridge: CartridgeProfile,
    timing: TimingProfile,
    capabilities: CapabilitySet,
}

impl TargetProfile {
    const fn from_validated_parts(
        id: TargetProfileId,
        family: TargetFamilyId,
        console: ConsoleModel,
        cartridge: CartridgeProfile,
        timing: TimingProfile,
        capabilities: CapabilitySet,
    ) -> Self {
        Self {
            id,
            family,
            console,
            cartridge,
            timing,
            capabilities,
        }
    }

    pub fn try_new(
        id: TargetProfileId,
        family: TargetFamilyId,
        console: ConsoleModel,
        cartridge: CartridgeProfile,
        timing: TimingProfile,
        capabilities: CapabilitySet,
    ) -> Result<Self, TargetProfileError> {
        let expected_id = canonical_target_profile_id(console, &cartridge, timing, capabilities);
        if id.as_str() != expected_id {
            return Err(TargetProfileError::TargetIdMismatch {
                expected: expected_id,
                actual: id.into_string(),
            });
        }
        let expected_family = console.expected_family_id();
        if family.as_str() != expected_family {
            return Err(TargetProfileError::TargetFamilyMismatch {
                expected: expected_family,
                actual: family.into_string(),
            });
        }
        if capabilities.rtc_present && !cartridge.has_rtc() {
            return Err(TargetProfileError::CapabilityRequiresRtcCartridge);
        }
        if capabilities.double_speed_mode && console != ConsoleModel::Cgb {
            return Err(TargetProfileError::CgbCapabilityOnNonCgb {
                console,
                capability: "double_speed_mode",
            });
        }
        if capabilities.double_speed_mode && timing.dots_per_m_cycle() != 2 {
            return Err(TargetProfileError::DoubleSpeedTimingMismatch {
                dots_per_m_cycle: timing.dots_per_m_cycle(),
            });
        }
        if capabilities.vram_dma && console != ConsoleModel::Cgb {
            return Err(TargetProfileError::CgbCapabilityOnNonCgb {
                console,
                capability: "vram_dma",
            });
        }

        Ok(Self {
            id,
            family,
            console,
            cartridge,
            timing,
            capabilities,
        })
    }

    #[must_use]
    pub fn id(&self) -> &TargetProfileId {
        &self.id
    }

    #[must_use]
    pub fn family(&self) -> &TargetFamilyId {
        &self.family
    }

    #[must_use]
    pub const fn console(&self) -> ConsoleModel {
        self.console
    }

    #[must_use]
    pub const fn cartridge(&self) -> &CartridgeProfile {
        &self.cartridge
    }

    #[must_use]
    pub const fn timing(&self) -> TimingProfile {
        self.timing
    }

    #[must_use]
    pub const fn capabilities(&self) -> CapabilitySet {
        self.capabilities
    }

    /// Return the deterministic content hash for this target profile.
    ///
    /// The hash is SHA-256 over [`TARGET_PROFILE_CONTENT_HASH_DOMAIN`] followed
    /// by sorted-key canonical JSON for the full public `TargetProfile` shape.
    pub fn content_hash(&self) -> Result<Hash256, serde_json::Error> {
        target_profile_content_hash(self)
    }
}

/// Return the deterministic content hash for a target profile.
///
/// See [`TARGET_PROFILE_CONTENT_HASH_DOMAIN`] for the domain-separated preimage
/// contract.
pub fn target_profile_content_hash(profile: &TargetProfile) -> Result<Hash256, serde_json::Error> {
    let value = serde_json::to_value(profile)?;
    let mut canonical_bytes = Vec::new();
    write_canonical_json_value(&value, &mut canonical_bytes)?;

    let mut hasher = Sha256::new();
    hasher.update(TARGET_PROFILE_CONTENT_HASH_DOMAIN);
    hasher.update(canonical_bytes);
    Ok(Hash256::from_bytes(hasher.finalize().into()))
}

fn write_canonical_json_value(
    value: &serde_json::Value,
    out: &mut Vec<u8>,
) -> Result<(), serde_json::Error> {
    match value {
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => serde_json::to_writer(out, value)?,
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (index, item) in items.iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                write_canonical_json_value(item, out)?;
            }
            out.push(b']');
        }
        serde_json::Value::Object(entries) => {
            out.push(b'{');
            let mut keys = entries.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for (index, key) in keys.into_iter().enumerate() {
                if index != 0 {
                    out.push(b',');
                }
                serde_json::to_writer(&mut *out, key)?;
                out.push(b':');
                write_canonical_json_value(&entries[key], out)?;
            }
            out.push(b'}');
        }
    }

    Ok(())
}

#[must_use]
pub const fn dmg_mbc5_8mib_128kib() -> TargetProfile {
    TargetProfile::from_validated_parts(
        TargetProfileId::from_static(BRINGUP_TARGET_PROFILE_ID),
        TargetFamilyId::from_static(DMG_TARGET_FAMILY_ID),
        ConsoleModel::Dmg,
        CartridgeProfile::dmg_mbc5_8mib_128kib_battery(),
        dmg_timing(),
        CapabilitySet {
            double_speed_mode: false,
            vram_dma: false,
            rtc_present: false,
        },
    )
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleModel {
    Dmg,
    Mgb,
    Sgb,
    Cgb,
}

impl ConsoleModel {
    #[must_use]
    pub const fn expected_family_id(self) -> &'static str {
        match self {
            Self::Dmg | Self::Mgb | Self::Sgb => DMG_TARGET_FAMILY_ID,
            Self::Cgb => "cgb",
        }
    }

    #[must_use]
    pub const fn profile_id_segment(self) -> &'static str {
        match self {
            Self::Dmg => "dmg",
            Self::Mgb => "mgb",
            Self::Sgb => "sgb",
            Self::Cgb => "cgb",
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(try_from = "CartridgeProfileRepr")]
pub struct CartridgeProfile {
    mbc_type: MbcType,
    rom_size: RomSize,
    ram_size: RamSize,
    has_battery: bool,
    has_rtc: bool,
}

#[derive(Copy, Clone, Debug, Deserialize)]
struct CartridgeProfileRepr {
    mbc_type: MbcType,
    rom_size: RomSize,
    ram_size: RamSize,
    has_battery: bool,
    has_rtc: bool,
}

impl TryFrom<CartridgeProfileRepr> for CartridgeProfile {
    type Error = TargetProfileError;

    fn try_from(value: CartridgeProfileRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.mbc_type,
            value.rom_size,
            value.ram_size,
            value.has_battery,
            value.has_rtc,
        )
    }
}

#[derive(Clone, Debug, Deserialize)]
struct TargetProfileRepr {
    id: TargetProfileId,
    family: TargetFamilyId,
    console: ConsoleModel,
    cartridge: CartridgeProfile,
    timing: TimingProfile,
    capabilities: CapabilitySet,
}

impl TryFrom<TargetProfileRepr> for TargetProfile {
    type Error = TargetProfileError;

    fn try_from(value: TargetProfileRepr) -> Result<Self, Self::Error> {
        Self::try_new(
            value.id,
            value.family,
            value.console,
            value.cartridge,
            value.timing,
            value.capabilities,
        )
    }
}

impl CartridgeProfile {
    #[must_use]
    pub const fn dmg_mbc5_8mib_128kib_battery() -> Self {
        Self {
            mbc_type: MbcType::Mbc5RamBattery,
            rom_size: RomSize::Mib8,
            ram_size: RamSize::Kib128,
            has_battery: true,
            has_rtc: false,
        }
    }

    pub fn try_new(
        mbc_type: MbcType,
        rom_size: RomSize,
        ram_size: RamSize,
        has_battery: bool,
        has_rtc: bool,
    ) -> Result<Self, TargetProfileError> {
        if !mbc_type.has_ram() && ram_size != RamSize::None {
            return Err(TargetProfileError::RamSizeWithoutRamCartridgeType {
                mbc_type,
                requested_kib: ram_size.kib(),
            });
        }

        let max_rom_kib = mbc_type.max_rom_kib();
        if rom_size.kib() > max_rom_kib {
            return Err(TargetProfileError::RomSizeExceedsMbcCapacity {
                mbc_type,
                max_rom_kib,
                requested_kib: rom_size.kib(),
            });
        }
        let max_sram_kib = mbc_type.max_sram_kib();
        if ram_size.kib() > max_sram_kib {
            return Err(TargetProfileError::SramSizeExceedsMbcCapacity {
                mbc_type,
                max_sram_kib,
                requested_kib: ram_size.kib(),
            });
        }
        if has_rtc {
            return Err(TargetProfileError::RtcWithoutMbc3 { mbc_type });
        }
        if mbc_type.has_ram() && ram_size == RamSize::None {
            return Err(TargetProfileError::CartridgeTypeRequiresRam { mbc_type });
        }
        if mbc_type.has_battery() != has_battery {
            return Err(TargetProfileError::BatteryFlagMismatch {
                mbc_type,
                expected: mbc_type.has_battery(),
                actual: has_battery,
            });
        }

        Ok(Self {
            mbc_type,
            rom_size,
            ram_size,
            has_battery,
            has_rtc,
        })
    }

    #[must_use]
    pub const fn mbc_type(&self) -> MbcType {
        self.mbc_type
    }

    #[must_use]
    pub const fn rom_size(&self) -> RomSize {
        self.rom_size
    }

    #[must_use]
    pub const fn ram_size(&self) -> RamSize {
        self.ram_size
    }

    #[must_use]
    pub const fn has_battery(&self) -> bool {
        self.has_battery
    }

    #[must_use]
    pub const fn has_rtc(&self) -> bool {
        self.has_rtc
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize, Default)]
pub struct CapabilitySet {
    pub double_speed_mode: bool,
    pub vram_dma: bool,
    pub rtc_present: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TargetProfileError {
    TargetIdMismatch {
        expected: String,
        actual: String,
    },
    TargetFamilyMismatch {
        expected: &'static str,
        actual: String,
    },
    RomSizeExceedsMbcCapacity {
        mbc_type: MbcType,
        max_rom_kib: u32,
        requested_kib: u32,
    },
    SramSizeExceedsMbcCapacity {
        mbc_type: MbcType,
        max_sram_kib: u32,
        requested_kib: u32,
    },
    RtcWithoutMbc3 {
        mbc_type: MbcType,
    },
    CartridgeTypeRequiresRam {
        mbc_type: MbcType,
    },
    RamSizeWithoutRamCartridgeType {
        mbc_type: MbcType,
        requested_kib: u32,
    },
    BatteryFlagMismatch {
        mbc_type: MbcType,
        expected: bool,
        actual: bool,
    },
    CapabilityRequiresRtcCartridge,
    CgbCapabilityOnNonCgb {
        console: ConsoleModel,
        capability: &'static str,
    },
    DoubleSpeedTimingMismatch {
        dots_per_m_cycle: u8,
    },
}

impl fmt::Display for TargetProfileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TargetIdMismatch { expected, actual } => {
                write!(f, "target id mismatch: expected {expected}, got {actual}")
            }
            Self::TargetFamilyMismatch { expected, actual } => {
                write!(
                    f,
                    "target family mismatch: expected {expected}, got {actual}"
                )
            }
            Self::RomSizeExceedsMbcCapacity {
                mbc_type,
                max_rom_kib,
                requested_kib,
            } => write!(
                f,
                "{mbc_type:?} supports at most {max_rom_kib} KiB ROM, requested {requested_kib} KiB"
            ),
            Self::SramSizeExceedsMbcCapacity {
                mbc_type,
                max_sram_kib,
                requested_kib,
            } => write!(
                f,
                "{mbc_type:?} supports at most {max_sram_kib} KiB SRAM, requested {requested_kib} KiB"
            ),
            Self::RtcWithoutMbc3 { mbc_type } => {
                write!(f, "RTC is not supported by {mbc_type:?}")
            }
            Self::CartridgeTypeRequiresRam { mbc_type } => {
                write!(f, "{mbc_type:?} requires a non-empty RAM size")
            }
            Self::RamSizeWithoutRamCartridgeType {
                mbc_type,
                requested_kib,
            } => write!(
                f,
                "{mbc_type:?} does not advertise RAM but requested {requested_kib} KiB"
            ),
            Self::BatteryFlagMismatch {
                mbc_type,
                expected,
                actual,
            } => write!(
                f,
                "{mbc_type:?} battery flag mismatch: expected {expected}, got {actual}"
            ),
            Self::CapabilityRequiresRtcCartridge => {
                f.write_str("rtc_present capability requires a cartridge with RTC")
            }
            Self::CgbCapabilityOnNonCgb {
                console,
                capability,
            } => write!(f, "{capability} is only valid on CGB, got {console:?}"),
            Self::DoubleSpeedTimingMismatch { dots_per_m_cycle } => write!(
                f,
                "double-speed targets must use two dots per M-cycle, got {dots_per_m_cycle}"
            ),
        }
    }
}

impl std::error::Error for TargetProfileError {}

#[must_use]
pub fn canonical_target_profile_id(
    console: ConsoleModel,
    cartridge: &CartridgeProfile,
    timing: TimingProfile,
    capabilities: CapabilitySet,
) -> String {
    if console == ConsoleModel::Dmg
        && *cartridge == CartridgeProfile::dmg_mbc5_8mib_128kib_battery()
        && timing == dmg_timing()
        && capabilities == CapabilitySet::default()
    {
        return BRINGUP_TARGET_PROFILE_ID.to_owned();
    }

    format!(
        "{}-{}-{}kib-rom-{}kib-sram-dot{}-dpm{}-frame{}-vblank{}-ds{}-vdma{}-rtc{}",
        console.profile_id_segment(),
        cartridge.mbc_type().profile_id_segment(),
        cartridge.rom_size().kib(),
        cartridge.ram_size().kib(),
        timing.dot_clock_hz(),
        timing.dots_per_m_cycle(),
        timing.frame_dots(),
        timing.vblank_dots(),
        capabilities.double_speed_mode,
        capabilities.vram_dma,
        capabilities.rtc_present,
    )
    .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dmg_mbc5_constructor() {
        let profile = dmg_mbc5_8mib_128kib();
        assert_eq!(profile.id().as_str(), BRINGUP_TARGET_PROFILE_ID);
        assert_eq!(profile.family().as_str(), DMG_TARGET_FAMILY_ID);
        assert_eq!(profile.console(), ConsoleModel::Dmg);
        assert_eq!(profile.cartridge().mbc_type(), MbcType::Mbc5RamBattery);
        assert_eq!(profile.cartridge().rom_size(), RomSize::Mib8);
        assert_eq!(profile.cartridge().ram_size(), RamSize::Kib128);
        assert_eq!(profile.timing(), dmg_timing());
        assert_eq!(profile.capabilities(), CapabilitySet::default());
    }

    #[test]
    fn serde_round_trip() {
        let profile = dmg_mbc5_8mib_128kib();
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: TargetProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn serde_json_shape_is_pinned() {
        let value = serde_json::to_value(dmg_mbc5_8mib_128kib()).unwrap();
        assert_eq!(
            value["console"],
            serde_json::json!("dmg"),
            "console casing is a public JSON contract"
        );
        assert_eq!(value["id"], serde_json::json!(BRINGUP_TARGET_PROFILE_ID));
        assert_eq!(value["family"], serde_json::json!(DMG_TARGET_FAMILY_ID));
        assert_eq!(
            value["cartridge"]["mbc_type"],
            serde_json::json!("mbc5_ram_battery")
        );
    }

    #[test]
    fn serde_rejects_invalid_target_profile() {
        let mut value = serde_json::to_value(dmg_mbc5_8mib_128kib()).unwrap();
        value["capabilities"]["double_speed_mode"] = serde_json::json!(true);
        assert!(serde_json::from_value::<TargetProfile>(value).is_err());
    }

    #[test]
    fn serde_rejects_invalid_cartridge_profile() {
        let bad = serde_json::json!({
            "mbc_type": "mbc5_ram_battery",
            "rom_size": "mib8",
            "ram_size": "kib128",
            "has_battery": false,
            "has_rtc": false
        });
        assert!(serde_json::from_value::<CartridgeProfile>(bad).is_err());
    }

    #[test]
    fn capability_set_exhaustive() {
        assert_eq!(
            CapabilitySet::default(),
            CapabilitySet {
                double_speed_mode: false,
                vram_dma: false,
                rtc_present: false,
            }
        );

        let invalid = CapabilitySet {
            double_speed_mode: true,
            vram_dma: true,
            rtc_present: true,
        };
        assert!(
            TargetProfile::try_new(
                TargetProfileId::from(BRINGUP_TARGET_PROFILE_ID),
                TargetFamilyId::from(DMG_TARGET_FAMILY_ID),
                ConsoleModel::Dmg,
                CartridgeProfile::dmg_mbc5_8mib_128kib_battery(),
                dmg_timing(),
                invalid,
            )
            .is_err()
        );
    }

    #[test]
    fn mbc_capacity_validation() {
        assert!(
            CartridgeProfile::try_new(
                MbcType::Mbc5RamBattery,
                RomSize::Mib8,
                RamSize::Kib128,
                true,
                false
            )
            .is_ok()
        );
        assert_eq!(
            CartridgeProfile::try_new(
                MbcType::Mbc5RamBattery,
                RomSize::Mib8,
                RamSize::Kib128,
                true,
                true
            ),
            Err(TargetProfileError::RtcWithoutMbc3 {
                mbc_type: MbcType::Mbc5RamBattery
            })
        );
        assert_eq!(
            CartridgeProfile::try_new(MbcType::Mbc5, RomSize::Kib32, RamSize::Kib8, false, false),
            Err(TargetProfileError::RamSizeWithoutRamCartridgeType {
                mbc_type: MbcType::Mbc5,
                requested_kib: 8,
            })
        );
    }

    #[test]
    fn id_is_content_addressed_by_stable_id_string() {
        let result = TargetProfile::try_new(
            TargetProfileId::from("wrong-id"),
            TargetFamilyId::from(DMG_TARGET_FAMILY_ID),
            ConsoleModel::Dmg,
            CartridgeProfile::dmg_mbc5_8mib_128kib_battery(),
            dmg_timing(),
            CapabilitySet::default(),
        );
        assert_eq!(
            result,
            Err(TargetProfileError::TargetIdMismatch {
                expected: BRINGUP_TARGET_PROFILE_ID.to_owned(),
                actual: "wrong-id".to_owned(),
            })
        );
    }

    #[test]
    fn generated_profile_ids_use_stable_segments() {
        let cartridge = CartridgeProfile::try_new(
            MbcType::Mbc5RamBattery,
            RomSize::Mib4,
            RamSize::Kib32,
            true,
            false,
        )
        .unwrap();
        let id = canonical_target_profile_id(
            ConsoleModel::Mgb,
            &cartridge,
            dmg_timing(),
            CapabilitySet::default(),
        );

        assert_eq!(
            id,
            "mgb-mbc5-ram-battery-4096kib-rom-32kib-sram-dot4194304-dpm4-frame70224-vblank4560-dsfalse-vdmafalse-rtcfalse"
        );
    }

    #[test]
    fn generated_profile_ids_include_full_timing_profile() {
        let cartridge = CartridgeProfile::try_new(
            MbcType::Mbc5Ram,
            RomSize::Mib4,
            RamSize::Kib32,
            false,
            false,
        )
        .unwrap();
        let canonical = canonical_target_profile_id(
            ConsoleModel::Dmg,
            &cartridge,
            dmg_timing(),
            CapabilitySet::default(),
        );
        let retimed = canonical_target_profile_id(
            ConsoleModel::Dmg,
            &cartridge,
            TimingProfile::try_new(4_194_305, 4, 70_224, 4_560).unwrap(),
            CapabilitySet::default(),
        );

        assert_ne!(canonical, retimed);
    }

    #[test]
    fn target_profile_content_hash_is_domain_separated_and_pinned() {
        assert_eq!(
            TARGET_PROFILE_CONTENT_HASH_DOMAIN,
            b"gbf:gbf-hw:TargetProfile:content_hash:1.0.0\0"
        );

        let profile = dmg_mbc5_8mib_128kib();
        let hash = profile.content_hash().expect("target profile hashes");
        assert_eq!(
            hash.to_string(),
            "64a347991811c5db12b7bc17dc2802d617b461c610ccde6ef81a22a1c28947c7"
        );

        let value = serde_json::to_value(&profile).expect("profile serializes");
        let mut canonical_bytes = Vec::new();
        write_canonical_json_value(&value, &mut canonical_bytes).expect("canonical JSON emits");
        let canonical_json =
            std::str::from_utf8(&canonical_bytes).expect("canonical JSON is UTF-8");
        assert!(
            canonical_json.starts_with(r#"{"capabilities":{"double_speed_mode":false"#),
            "top-level and nested object keys must be lexicographically ordered"
        );

        let no_domain_digest = Sha256::digest(canonical_bytes);
        assert_ne!(
            hash,
            Hash256::from_bytes(no_domain_digest.into()),
            "content_hash must include the explicit TargetProfile domain separator"
        );
        assert_ne!(
            target_profile_content_hash(
                &TargetProfile::try_new(
                    TargetProfileId::from(canonical_target_profile_id(
                        ConsoleModel::Cgb,
                        profile.cartridge(),
                        TimingProfile::try_new(4_194_304, 2, 70_224, 4_560).unwrap(),
                        CapabilitySet {
                            double_speed_mode: true,
                            vram_dma: true,
                            rtc_present: false,
                        },
                    )),
                    TargetFamilyId::from("cgb"),
                    ConsoleModel::Cgb,
                    *profile.cartridge(),
                    TimingProfile::try_new(4_194_304, 2, 70_224, 4_560).unwrap(),
                    CapabilitySet {
                        double_speed_mode: true,
                        vram_dma: true,
                        rtc_present: false,
                    },
                )
                .unwrap(),
            )
            .unwrap(),
            hash,
            "target-relevant content changes must change the content hash"
        );
    }

    #[test]
    fn family_id_groups_by_console_arch() {
        assert_eq!(dmg_mbc5_8mib_128kib().family().as_str(), "dmg");
        let result = TargetProfile::try_new(
            TargetProfileId::from(BRINGUP_TARGET_PROFILE_ID),
            TargetFamilyId::from("cgb"),
            ConsoleModel::Dmg,
            CartridgeProfile::dmg_mbc5_8mib_128kib_battery(),
            dmg_timing(),
            CapabilitySet::default(),
        );
        assert_eq!(
            result,
            Err(TargetProfileError::TargetFamilyMismatch {
                expected: DMG_TARGET_FAMILY_ID,
                actual: "cgb".to_owned(),
            })
        );
    }

    #[test]
    fn error_variants_are_typed() {
        let err = TargetProfileError::BatteryFlagMismatch {
            mbc_type: MbcType::Mbc5RamBattery,
            expected: true,
            actual: false,
        };
        assert!(err.to_string().contains("battery flag mismatch"));
    }
}
