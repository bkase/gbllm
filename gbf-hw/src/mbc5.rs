//! MBC5 cartridge memory-controller write-address semantics.
//!
//! `gbf-hw` intentionally exposes only the canonical `$0A` RAM-enable byte,
//! not a loose predicate for "any low nibble A" values:
//!
//! ```compile_fail
//! use gbf_hw::mbc5::is_ram_enable_value;
//! ```

use serde::{Deserialize, Serialize};

pub const MBC5_RAMG_BASE: u16 = 0x0000;
pub const MBC5_RAMG_END: u16 = 0x1FFF;
pub const MBC5_BANK1_BASE: u16 = 0x2000;
pub const MBC5_BANK1_END: u16 = 0x2FFF;
pub const MBC5_BANK2_BASE: u16 = 0x3000;
pub const MBC5_BANK2_END: u16 = 0x3FFF;
pub const MBC5_RAMB_BASE: u16 = 0x4000;
pub const MBC5_RAMB_END: u16 = 0x5FFF;
pub const MBC5_RESERVED_BASE: u16 = 0x6000;
pub const MBC5_RESERVED_END: u16 = 0x7FFF;

/// Canonical MBC5 SRAM-enable value.
///
/// Pan Docs notes that other values with `$A` in the low nibble may enable
/// cartridge RAM on some hardware, but relying on those values is not
/// recommended for compatibility. `gbf-hw` intentionally exposes only `$0A`.
pub const MBC5_RAM_ENABLE_VALUE: u8 = 0x0A;

/// Canonical MBC5 SRAM-disable value. Any non-enable value disables SRAM; `$00`
/// is the canonical byte emitted by runtime banking code.
pub const MBC5_RAM_DISABLE_VALUE: u8 = 0x00;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MbcRegisterClass {
    Ramg,
    Bank1,
    Bank2,
    Ramb,
    Reserved,
}

#[must_use]
pub const fn classify_mbc_write_address(addr: u16) -> Option<MbcRegisterClass> {
    match addr {
        MBC5_RAMG_BASE..=MBC5_RAMG_END => Some(MbcRegisterClass::Ramg),
        MBC5_BANK1_BASE..=MBC5_BANK1_END => Some(MbcRegisterClass::Bank1),
        MBC5_BANK2_BASE..=MBC5_BANK2_END => Some(MbcRegisterClass::Bank2),
        MBC5_RAMB_BASE..=MBC5_RAMB_END => Some(MbcRegisterClass::Ramb),
        MBC5_RESERVED_BASE..=MBC5_RESERVED_END => Some(MbcRegisterClass::Reserved),
        _ => None,
    }
}

#[must_use]
/// Assemble the MBC5 9-bit ROM bank number from BANK1 and BANK2 writes.
///
/// Returns a bank number in `0..=511`, not a CPU address. Only bit 0 of BANK2
/// participates. Unlike MBC1, MBC5 does not remap bank 0 to bank 1.
pub const fn rom_bank_number(bank1: u8, bank2: u8) -> u16 {
    ((bank2 as u16 & 0x01) << 8) | bank1 as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_address_classification() {
        for (start, end, class) in [
            (MBC5_RAMG_BASE, MBC5_RAMG_END, MbcRegisterClass::Ramg),
            (MBC5_BANK1_BASE, MBC5_BANK1_END, MbcRegisterClass::Bank1),
            (MBC5_BANK2_BASE, MBC5_BANK2_END, MbcRegisterClass::Bank2),
            (MBC5_RAMB_BASE, MBC5_RAMB_END, MbcRegisterClass::Ramb),
            (
                MBC5_RESERVED_BASE,
                MBC5_RESERVED_END,
                MbcRegisterClass::Reserved,
            ),
        ] {
            for addr in start..=end {
                assert_eq!(
                    classify_mbc_write_address(addr),
                    Some(class),
                    "bad MBC5 class for {addr:#06x}"
                );
            }
        }
    }

    #[test]
    fn address_outside_rom_returns_none() {
        assert_eq!(classify_mbc_write_address(0x8000), None);
        assert_eq!(classify_mbc_write_address(0xFFFF), None);
    }

    #[test]
    fn ram_enable_value() {
        assert_eq!(MBC5_RAM_ENABLE_VALUE, 0x0A);
        assert_eq!(MBC5_RAM_DISABLE_VALUE, 0x00);
    }

    #[test]
    fn bank_number_assembly() {
        assert_eq!(rom_bank_number(0x00, 0x00), 0x000);
        assert_eq!(rom_bank_number(0xFF, 0x01), 0x1FF);
        assert_eq!(rom_bank_number(0x42, 0x00), 0x042);
    }

    #[test]
    fn bank_number_high_bit_only_uses_lsb() {
        assert_eq!(rom_bank_number(0x77, 0xFE), rom_bank_number(0x77, 0x00));
        assert_eq!(rom_bank_number(0x77, 0xFF), rom_bank_number(0x77, 0x01));
    }

    #[test]
    fn reserved_band_is_named() {
        assert_eq!(
            classify_mbc_write_address(0x7000),
            Some(MbcRegisterClass::Reserved)
        );
    }
}
