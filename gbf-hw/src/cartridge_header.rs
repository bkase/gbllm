//! Cartridge-header constants and typed header-byte tables.

use serde::{Deserialize, Serialize};

/// Nintendo logo bytes stored at cartridge header range `$0104..=$0133`.
///
/// Pan Docs: The Cartridge Header.
pub const NINTENDO_LOGO: [u8; 48] = [
    0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C, 0x00, 0x0D,
    0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99,
    0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
];

/// Cartridge type byte values for the MBC5 cartridges supported in M0.
///
/// Pan Docs: The Cartridge Header, cartridge type `$0147`.
///
/// This is intentionally the MBC5 subset that F-A1 shipped. Rumble, sensor,
/// and non-MBC5 cartridge families are additive follow-up variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MbcType {
    Mbc5,
    Mbc5Ram,
    Mbc5RamBattery,
}

impl MbcType {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Mbc5 => 0x19,
            Self::Mbc5Ram => 0x1A,
            Self::Mbc5RamBattery => 0x1B,
        }
    }

    #[must_use]
    pub const fn has_ram(self) -> bool {
        matches!(self, Self::Mbc5Ram | Self::Mbc5RamBattery)
    }

    #[must_use]
    pub const fn has_battery(self) -> bool {
        matches!(self, Self::Mbc5RamBattery)
    }

    #[must_use]
    pub const fn max_rom_kib(self) -> u32 {
        8 * 1024
    }

    #[must_use]
    pub const fn max_sram_kib(self) -> u32 {
        if self.has_ram() { 128 } else { 0 }
    }

    #[must_use]
    pub const fn profile_id_segment(self) -> &'static str {
        match self {
            Self::Mbc5 => "mbc5",
            Self::Mbc5Ram => "mbc5-ram",
            Self::Mbc5RamBattery => "mbc5-ram-battery",
        }
    }
}

/// ROM size byte values for cartridge header offset `$0148`.
///
/// Pan Docs: The Cartridge Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RomSize {
    Kib32,
    Kib64,
    Kib128,
    Kib256,
    Kib512,
    Mib1,
    Mib2,
    Mib4,
    Mib8,
}

impl RomSize {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Kib32 => 0x00,
            Self::Kib64 => 0x01,
            Self::Kib128 => 0x02,
            Self::Kib256 => 0x03,
            Self::Kib512 => 0x04,
            Self::Mib1 => 0x05,
            Self::Mib2 => 0x06,
            Self::Mib4 => 0x07,
            Self::Mib8 => 0x08,
        }
    }

    #[must_use]
    pub const fn bank_count(self) -> u16 {
        match self {
            Self::Kib32 => 2,
            Self::Kib64 => 4,
            Self::Kib128 => 8,
            Self::Kib256 => 16,
            Self::Kib512 => 32,
            Self::Mib1 => 64,
            Self::Mib2 => 128,
            Self::Mib4 => 256,
            Self::Mib8 => 512,
        }
    }

    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bank_count() as usize * 16 * 1024
    }

    #[must_use]
    pub const fn kib(self) -> u32 {
        32_u32 << self.header_byte()
    }
}

/// SRAM size byte values for cartridge header offset `$0149`.
///
/// Pan Docs: The Cartridge Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RamSize {
    None,
    Kib8,
    Kib32,
    Kib128,
    Kib64,
}

impl RamSize {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::None => 0x00,
            Self::Kib8 => 0x02,
            Self::Kib32 => 0x03,
            Self::Kib128 => 0x04,
            Self::Kib64 => 0x05,
        }
    }

    #[must_use]
    pub const fn kib(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Kib8 => 8,
            Self::Kib32 => 32,
            Self::Kib128 => 128,
            Self::Kib64 => 64,
        }
    }

    #[must_use]
    pub const fn bank_count(self) -> u16 {
        (self.kib() / 8) as u16
    }
}

/// Destination code byte values for cartridge header offset `$014A`.
///
/// Pan Docs: The Cartridge Header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DestinationCode {
    Japan,
    Overseas,
}

impl DestinationCode {
    #[must_use]
    pub const fn header_byte(self) -> u8 {
        match self {
            Self::Japan => 0x00,
            Self::Overseas => 0x01,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nintendo_logo_first_byte() {
        assert_eq!(NINTENDO_LOGO[0], 0xCE);
    }

    #[test]
    fn nintendo_logo_length() {
        assert_eq!(NINTENDO_LOGO.len(), 48);
    }

    #[test]
    fn nintendo_logo_known_vector() {
        assert_eq!(
            NINTENDO_LOGO,
            [
                0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83, 0x00, 0x0C,
                0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E, 0xDC, 0xCC, 0x6E, 0xE6,
                0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63, 0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC,
                0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
            ]
        );
    }

    #[test]
    fn mbc_type_header_bytes() {
        assert_eq!(MbcType::Mbc5.header_byte(), 0x19);
        assert_eq!(MbcType::Mbc5Ram.header_byte(), 0x1A);
        assert_eq!(MbcType::Mbc5RamBattery.header_byte(), 0x1B);
    }

    #[test]
    fn mbc_type_capacity_limits() {
        assert_eq!(MbcType::Mbc5.max_rom_kib(), 8 * 1024);
        assert_eq!(MbcType::Mbc5.max_sram_kib(), 0);
        assert_eq!(MbcType::Mbc5Ram.max_sram_kib(), 128);
        assert_eq!(
            MbcType::Mbc5RamBattery.profile_id_segment(),
            "mbc5-ram-battery"
        );
    }

    #[test]
    fn rom_size_kib_formula() {
        let sizes = [
            RomSize::Kib32,
            RomSize::Kib64,
            RomSize::Kib128,
            RomSize::Kib256,
            RomSize::Kib512,
            RomSize::Mib1,
            RomSize::Mib2,
            RomSize::Mib4,
            RomSize::Mib8,
        ];

        for size in sizes {
            assert_eq!(size.kib(), 32 << size.header_byte());
        }
    }

    #[test]
    fn rom_size_bank_count() {
        assert_eq!(RomSize::Mib8.bank_count(), 512);
    }

    #[test]
    fn rom_size_bytes_consistent() {
        for size in [
            RomSize::Kib32,
            RomSize::Kib64,
            RomSize::Kib128,
            RomSize::Kib256,
            RomSize::Kib512,
            RomSize::Mib1,
            RomSize::Mib2,
            RomSize::Mib4,
            RomSize::Mib8,
        ] {
            assert_eq!(size.kib() as usize * 1024, size.bytes());
        }
    }

    #[test]
    fn ram_size_kib_table() {
        assert_eq!(RamSize::None.kib(), 0);
        assert_eq!(RamSize::Kib8.kib(), 8);
        assert_eq!(RamSize::Kib32.kib(), 32);
        assert_eq!(RamSize::Kib128.kib(), 128);
        assert_eq!(RamSize::Kib64.kib(), 64);
    }

    #[test]
    fn ram_size_64kib_after_128kib() {
        assert_eq!(RamSize::Kib128.header_byte(), 0x04);
        assert_eq!(RamSize::Kib64.header_byte(), 0x05);
    }

    #[test]
    fn destination_code_header_bytes() {
        assert_eq!(DestinationCode::Japan.header_byte(), 0x00);
        assert_eq!(DestinationCode::Overseas.header_byte(), 0x01);
    }

    #[test]
    fn serde_snake_case_round_trip() {
        let encoded = serde_json::to_string(&MbcType::Mbc5RamBattery).unwrap();
        assert_eq!(encoded, r#""mbc5_ram_battery""#);
        assert_eq!(
            serde_json::from_str::<MbcType>(&encoded).unwrap(),
            MbcType::Mbc5RamBattery
        );
    }

    #[test]
    fn serde_unknown_variants_rejected() {
        assert!(serde_json::from_str::<RomSize>(r#""kib16""#).is_err());
        assert!(serde_json::from_str::<RamSize>(r#""kib16""#).is_err());
        assert!(serde_json::from_str::<MbcType>(r#""mbc1""#).is_err());
    }
}
