//! LCD register constants and PPU accessibility predicates.

use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum PpuMode {
    HBlank = 0,
    VBlank = 1,
    OamSearch = 2,
    Drawing = 3,
}

impl PpuMode {
    pub const ALL: [PpuMode; 4] = [
        PpuMode::HBlank,
        PpuMode::VBlank,
        PpuMode::OamSearch,
        PpuMode::Drawing,
    ];

    #[must_use]
    pub const fn from_stat_bits(bits: u8) -> Self {
        match bits & 0b11 {
            0 => Self::HBlank,
            1 => Self::VBlank,
            2 => Self::OamSearch,
            _ => Self::Drawing,
        }
    }

    #[must_use]
    pub const fn to_stat_bits(self) -> u8 {
        self as u8
    }
}

pub const LCDC_REG: u16 = crate::memory::io_register(0x40);
pub const STAT_REG: u16 = crate::memory::io_register(0x41);
pub const SCY_REG: u16 = crate::memory::io_register(0x42);
pub const SCX_REG: u16 = crate::memory::io_register(0x43);
pub const LY_REG: u16 = crate::memory::io_register(0x44);
pub const LYC_REG: u16 = crate::memory::io_register(0x45);
pub const DMA_REG: u16 = crate::memory::io_register(0x46);
pub const BGP_REG: u16 = crate::memory::io_register(0x47);
pub const OBP0_REG: u16 = crate::memory::io_register(0x48);
pub const OBP1_REG: u16 = crate::memory::io_register(0x49);
pub const WY_REG: u16 = crate::memory::io_register(0x4A);
pub const WX_REG: u16 = crate::memory::io_register(0x4B);

pub const STAT_INTERRUPT_LYC_ENABLE: u8 = 0b0100_0000;
pub const STAT_INTERRUPT_OAM_ENABLE: u8 = 0b0010_0000;
pub const STAT_INTERRUPT_VBLANK_ENABLE: u8 = 0b0001_0000;
pub const STAT_INTERRUPT_HBLANK_ENABLE: u8 = 0b0000_1000;
pub const STAT_INTERRUPT_ENABLE_MASK: u8 = STAT_INTERRUPT_LYC_ENABLE
    | STAT_INTERRUPT_OAM_ENABLE
    | STAT_INTERRUPT_VBLANK_ENABLE
    | STAT_INTERRUPT_HBLANK_ENABLE;

pub const SCREEN_WIDTH_PIXELS: u8 = 160;
pub const SCREEN_HEIGHT_PIXELS: u8 = 144;
pub const VBLANK_FIRST_LY: u8 = SCREEN_HEIGHT_PIXELS;
pub const VBLANK_LAST_LY: u8 = 153;
pub const VBLANK_LY_THRESHOLD: u8 = VBLANK_FIRST_LY;

#[must_use]
pub const fn vram_accessible_in(mode: PpuMode) -> bool {
    matches!(mode, PpuMode::HBlank | PpuMode::VBlank | PpuMode::OamSearch)
}

#[must_use]
pub const fn oam_accessible_in(mode: PpuMode) -> bool {
    matches!(mode, PpuMode::HBlank | PpuMode::VBlank)
}

#[must_use]
pub const fn vram_accessible(lcd_enabled: bool, mode: PpuMode) -> bool {
    !lcd_enabled || vram_accessible_in(mode)
}

#[must_use]
pub const fn oam_accessible(lcd_enabled: bool, mode: PpuMode) -> bool {
    !lcd_enabled || oam_accessible_in(mode)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory;

    #[test]
    fn vram_oam_accessibility_table() {
        assert!(vram_accessible_in(PpuMode::HBlank));
        assert!(oam_accessible_in(PpuMode::HBlank));
        assert!(vram_accessible_in(PpuMode::VBlank));
        assert!(oam_accessible_in(PpuMode::VBlank));
        assert!(vram_accessible_in(PpuMode::OamSearch));
        assert!(!oam_accessible_in(PpuMode::OamSearch));
        assert!(!vram_accessible_in(PpuMode::Drawing));
        assert!(!oam_accessible_in(PpuMode::Drawing));
    }

    #[test]
    fn lcd_disabled_unrestricted() {
        for mode in PpuMode::ALL {
            assert!(vram_accessible(false, mode));
            assert!(oam_accessible(false, mode));
        }
    }

    #[test]
    fn from_stat_bits_round_trip() {
        for mode in PpuMode::ALL {
            assert_eq!(PpuMode::from_stat_bits(mode.to_stat_bits()), mode);
            assert_eq!(
                PpuMode::from_stat_bits(mode.to_stat_bits() | 0b1111_1100),
                mode
            );
        }
    }

    #[test]
    fn ppu_mode_discriminants() {
        assert_eq!(PpuMode::HBlank as u8, 0);
        assert_eq!(PpuMode::VBlank as u8, 1);
        assert_eq!(PpuMode::OamSearch as u8, 2);
        assert_eq!(PpuMode::Drawing as u8, 3);
    }

    #[test]
    fn vblank_ly_threshold() {
        assert_eq!(SCREEN_WIDTH_PIXELS, 160);
        assert_eq!(SCREEN_HEIGHT_PIXELS, 144);
        assert_eq!(VBLANK_FIRST_LY, 144);
        assert_eq!(VBLANK_LAST_LY, 153);
        assert_eq!(VBLANK_LY_THRESHOLD, 144);
    }

    #[test]
    fn register_addresses_in_io_region() {
        for reg in [
            LCDC_REG, STAT_REG, SCY_REG, SCX_REG, LY_REG, LYC_REG, DMA_REG, BGP_REG, OBP0_REG,
            OBP1_REG, WY_REG, WX_REG,
        ] {
            assert!(memory::is_io(reg), "LCD register outside IO: {reg:#06x}");
        }
    }

    #[test]
    fn register_addresses_are_exact() {
        assert_eq!(LCDC_REG, 0xFF40);
        assert_eq!(STAT_REG, 0xFF41);
        assert_eq!(SCY_REG, 0xFF42);
        assert_eq!(SCX_REG, 0xFF43);
        assert_eq!(LY_REG, 0xFF44);
        assert_eq!(LYC_REG, 0xFF45);
        assert_eq!(DMA_REG, 0xFF46);
        assert_eq!(BGP_REG, 0xFF47);
        assert_eq!(OBP0_REG, 0xFF48);
        assert_eq!(OBP1_REG, 0xFF49);
        assert_eq!(WY_REG, 0xFF4A);
        assert_eq!(WX_REG, 0xFF4B);
    }

    #[test]
    fn stat_interrupt_enable_bits_are_writable_upper_bits() {
        assert_eq!(STAT_INTERRUPT_HBLANK_ENABLE, 0x08);
        assert_eq!(STAT_INTERRUPT_VBLANK_ENABLE, 0x10);
        assert_eq!(STAT_INTERRUPT_OAM_ENABLE, 0x20);
        assert_eq!(STAT_INTERRUPT_LYC_ENABLE, 0x40);
        assert_eq!(STAT_INTERRUPT_ENABLE_MASK & 0b1000_0111, 0);
    }
}
