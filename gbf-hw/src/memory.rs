//! Game Boy CPU memory-map constants and predicates.

use serde::{Deserialize, Serialize};

pub const ROM_BANK0_BASE: u16 = 0x0000;
pub const ROM_BANK0_END: u16 = 0x3FFF;
pub const ROM_SWITCHABLE_BASE: u16 = 0x4000;
pub const ROM_SWITCHABLE_END: u16 = 0x7FFF;
pub const VRAM_BASE: u16 = 0x8000;
pub const VRAM_END: u16 = 0x9FFF;
pub const SRAM_BASE: u16 = 0xA000;
pub const SRAM_END: u16 = 0xBFFF;
pub const WRAM0_BASE: u16 = 0xC000;
pub const WRAM0_END: u16 = 0xCFFF;
pub const WRAMX_BASE: u16 = 0xD000;
pub const WRAMX_END: u16 = 0xDFFF;
pub const WRAM_BASE: u16 = WRAM0_BASE;
pub const WRAM_END: u16 = WRAMX_END;
pub const ECHO_RAM_BASE: u16 = 0xE000;
pub const ECHO_RAM_END: u16 = 0xFDFF;
pub const OAM_BASE: u16 = 0xFE00;
pub const OAM_END: u16 = 0xFE9F;
pub const UNMAPPED_BASE: u16 = 0xFEA0;
pub const UNMAPPED_END: u16 = 0xFEFF;
pub const IO_BASE: u16 = 0xFF00;
pub const IO_END: u16 = 0xFF7F;
pub const HRAM_BASE: u16 = 0xFF80;
pub const HRAM_END: u16 = 0xFFFE;
pub const IE_REG: u16 = 0xFFFF;

pub const BANK0_SIZE_BYTES: u32 = 16 * 1024;
pub const SWITCHABLE_BANK_SIZE_BYTES: u32 = 16 * 1024;
pub const SRAM_BANK_SIZE_BYTES: u32 = 8 * 1024;
pub const WRAM0_SIZE_BYTES: u32 = 4 * 1024;
pub const WRAMX_SIZE_BYTES: u32 = 4 * 1024;
pub const WRAM_SIZE_BYTES: u32 = WRAM0_SIZE_BYTES + WRAMX_SIZE_BYTES;
pub const VRAM_SIZE_BYTES: u32 = 8 * 1024;
pub const HRAM_SIZE_BYTES: u32 = 127;
pub const OAM_SIZE_BYTES: u32 = 160;
pub const IO_SIZE_BYTES: u32 = 128;
pub const ECHO_RAM_SIZE_BYTES: u32 = ECHO_RAM_END as u32 - ECHO_RAM_BASE as u32 + 1;
pub const UNMAPPED_SIZE_BYTES: u32 = UNMAPPED_END as u32 - UNMAPPED_BASE as u32 + 1;

#[must_use]
pub const fn io_register(offset: u8) -> u16 {
    IO_BASE + offset as u16
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MemoryRegion {
    RomBank0,
    RomSwitchable,
    Vram,
    Sram,
    Wram0,
    WramX,
    EchoRam,
    Oam,
    Unmapped,
    Io,
    Hram,
    InterruptEnable,
}

#[must_use]
pub const fn classify(addr: u16) -> MemoryRegion {
    match addr {
        ROM_BANK0_BASE..=ROM_BANK0_END => MemoryRegion::RomBank0,
        ROM_SWITCHABLE_BASE..=ROM_SWITCHABLE_END => MemoryRegion::RomSwitchable,
        VRAM_BASE..=VRAM_END => MemoryRegion::Vram,
        SRAM_BASE..=SRAM_END => MemoryRegion::Sram,
        WRAM0_BASE..=WRAM0_END => MemoryRegion::Wram0,
        WRAMX_BASE..=WRAMX_END => MemoryRegion::WramX,
        ECHO_RAM_BASE..=ECHO_RAM_END => MemoryRegion::EchoRam,
        OAM_BASE..=OAM_END => MemoryRegion::Oam,
        UNMAPPED_BASE..=UNMAPPED_END => MemoryRegion::Unmapped,
        IO_BASE..=IO_END => MemoryRegion::Io,
        HRAM_BASE..=HRAM_END => MemoryRegion::Hram,
        IE_REG => MemoryRegion::InterruptEnable,
    }
}

#[must_use]
pub const fn is_rom_bank0(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::RomBank0)
}

#[must_use]
pub const fn is_rom_switchable(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::RomSwitchable)
}

#[must_use]
pub const fn is_vram(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Vram)
}

#[must_use]
pub const fn is_sram_window(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Sram)
}

#[must_use]
pub const fn is_wram(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Wram0 | MemoryRegion::WramX)
}

#[must_use]
pub const fn is_fixed_wram_dmg(addr: u16) -> bool {
    is_wram(addr)
}

#[must_use]
pub const fn is_fixed_wram_cgb(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Wram0)
}

#[must_use]
pub const fn is_oam(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Oam)
}

#[must_use]
pub const fn is_io(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Io)
}

#[must_use]
pub const fn is_hram(addr: u16) -> bool {
    matches!(classify(addr), MemoryRegion::Hram)
}

#[must_use]
pub const fn is_isr_resident_legal_dmg(addr: u16) -> bool {
    matches!(
        classify(addr),
        MemoryRegion::RomBank0
            | MemoryRegion::Wram0
            | MemoryRegion::WramX
            | MemoryRegion::Hram
            | MemoryRegion::InterruptEnable
    )
}

#[must_use]
pub const fn is_isr_resident_legal_cgb(addr: u16) -> bool {
    matches!(
        classify(addr),
        MemoryRegion::RomBank0
            | MemoryRegion::Wram0
            | MemoryRegion::Hram
            | MemoryRegion::InterruptEnable
    )
}

#[must_use]
pub const fn is_isr_io_register_allowed(addr: u16) -> bool {
    matches!(
        addr,
        crate::interrupts::IF_REGISTER | crate::interrupts::IE_REGISTER
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interrupts::{IE_REGISTER, IF_REGISTER};

    fn region_contains(region: MemoryRegion, addr: u16) -> bool {
        match region {
            MemoryRegion::RomBank0 => (ROM_BANK0_BASE..=ROM_BANK0_END).contains(&addr),
            MemoryRegion::RomSwitchable => {
                (ROM_SWITCHABLE_BASE..=ROM_SWITCHABLE_END).contains(&addr)
            }
            MemoryRegion::Vram => (VRAM_BASE..=VRAM_END).contains(&addr),
            MemoryRegion::Sram => (SRAM_BASE..=SRAM_END).contains(&addr),
            MemoryRegion::Wram0 => (WRAM0_BASE..=WRAM0_END).contains(&addr),
            MemoryRegion::WramX => (WRAMX_BASE..=WRAMX_END).contains(&addr),
            MemoryRegion::EchoRam => (ECHO_RAM_BASE..=ECHO_RAM_END).contains(&addr),
            MemoryRegion::Oam => (OAM_BASE..=OAM_END).contains(&addr),
            MemoryRegion::Unmapped => (UNMAPPED_BASE..=UNMAPPED_END).contains(&addr),
            MemoryRegion::Io => (IO_BASE..=IO_END).contains(&addr),
            MemoryRegion::Hram => (HRAM_BASE..=HRAM_END).contains(&addr),
            MemoryRegion::InterruptEnable => addr == IE_REG,
        }
    }

    #[test]
    fn region_classification() {
        for addr in u16::MIN..=u16::MAX {
            let region = classify(addr);
            assert!(
                region_contains(region, addr),
                "bad classification for {addr:#06x}"
            );
        }
    }

    #[test]
    fn pan_docs_boundary_literals() {
        let boundaries = [
            (ROM_BANK0_BASE, 0x0000, MemoryRegion::RomBank0),
            (ROM_BANK0_END, 0x3FFF, MemoryRegion::RomBank0),
            (ROM_SWITCHABLE_BASE, 0x4000, MemoryRegion::RomSwitchable),
            (ROM_SWITCHABLE_END, 0x7FFF, MemoryRegion::RomSwitchable),
            (VRAM_BASE, 0x8000, MemoryRegion::Vram),
            (VRAM_END, 0x9FFF, MemoryRegion::Vram),
            (SRAM_BASE, 0xA000, MemoryRegion::Sram),
            (SRAM_END, 0xBFFF, MemoryRegion::Sram),
            (WRAM0_BASE, 0xC000, MemoryRegion::Wram0),
            (WRAM0_END, 0xCFFF, MemoryRegion::Wram0),
            (WRAMX_BASE, 0xD000, MemoryRegion::WramX),
            (WRAMX_END, 0xDFFF, MemoryRegion::WramX),
            (ECHO_RAM_BASE, 0xE000, MemoryRegion::EchoRam),
            (ECHO_RAM_END, 0xFDFF, MemoryRegion::EchoRam),
            (OAM_BASE, 0xFE00, MemoryRegion::Oam),
            (OAM_END, 0xFE9F, MemoryRegion::Oam),
            (UNMAPPED_BASE, 0xFEA0, MemoryRegion::Unmapped),
            (UNMAPPED_END, 0xFEFF, MemoryRegion::Unmapped),
            (IO_BASE, 0xFF00, MemoryRegion::Io),
            (IO_END, 0xFF7F, MemoryRegion::Io),
            (HRAM_BASE, 0xFF80, MemoryRegion::Hram),
            (HRAM_END, 0xFFFE, MemoryRegion::Hram),
            (IE_REG, 0xFFFF, MemoryRegion::InterruptEnable),
        ];

        for (constant, literal, region) in boundaries {
            assert_eq!(constant, literal);
            assert_eq!(classify(literal), region);
        }
    }

    #[test]
    fn isr_resident_legal_dmg() {
        assert!(is_isr_resident_legal_dmg(ROM_BANK0_BASE));
        assert!(is_isr_resident_legal_dmg(WRAMX_BASE));
        assert!(is_isr_resident_legal_dmg(HRAM_BASE));
        assert!(!is_isr_resident_legal_dmg(SRAM_BASE));
        assert!(!is_isr_resident_legal_dmg(IO_BASE));
    }

    #[test]
    fn isr_resident_legal_cgb() {
        assert!(is_isr_resident_legal_cgb(ROM_BANK0_BASE));
        assert!(is_isr_resident_legal_cgb(WRAM0_BASE));
        assert!(!is_isr_resident_legal_cgb(WRAMX_BASE));
    }

    #[test]
    fn isr_io_register_allowed() {
        assert!(is_isr_io_register_allowed(IF_REGISTER));
        assert!(is_isr_io_register_allowed(IE_REGISTER));
        assert!(!is_isr_io_register_allowed(IO_BASE));
    }

    #[test]
    fn region_sizes() {
        assert_eq!(
            BANK0_SIZE_BYTES,
            ROM_BANK0_END as u32 - ROM_BANK0_BASE as u32 + 1
        );
        assert_eq!(
            SWITCHABLE_BANK_SIZE_BYTES,
            ROM_SWITCHABLE_END as u32 - ROM_SWITCHABLE_BASE as u32 + 1
        );
        assert_eq!(SRAM_BANK_SIZE_BYTES, SRAM_END as u32 - SRAM_BASE as u32 + 1);
        assert_eq!(WRAM0_SIZE_BYTES, WRAM0_END as u32 - WRAM0_BASE as u32 + 1);
        assert_eq!(WRAMX_SIZE_BYTES, WRAMX_END as u32 - WRAMX_BASE as u32 + 1);
        assert_eq!(VRAM_SIZE_BYTES, VRAM_END as u32 - VRAM_BASE as u32 + 1);
        assert_eq!(HRAM_SIZE_BYTES, HRAM_END as u32 - HRAM_BASE as u32 + 1);
        assert_eq!(OAM_SIZE_BYTES, OAM_END as u32 - OAM_BASE as u32 + 1);
        assert_eq!(IO_SIZE_BYTES, IO_END as u32 - IO_BASE as u32 + 1);
        assert_eq!(
            ECHO_RAM_SIZE_BYTES,
            ECHO_RAM_END as u32 - ECHO_RAM_BASE as u32 + 1
        );
        assert_eq!(
            UNMAPPED_SIZE_BYTES,
            UNMAPPED_END as u32 - UNMAPPED_BASE as u32 + 1
        );
        assert_eq!(WRAM_SIZE_BYTES, WRAM0_SIZE_BYTES + WRAMX_SIZE_BYTES);
    }

    #[test]
    fn predicate_totality() {
        for addr in u16::MIN..=u16::MAX {
            let _ = is_rom_bank0(addr);
            let _ = is_rom_switchable(addr);
            let _ = is_vram(addr);
            let _ = is_sram_window(addr);
            let _ = is_wram(addr);
            let _ = is_oam(addr);
            let _ = is_io(addr);
            let _ = is_hram(addr);
        }
    }

    #[test]
    fn no_predicate_overlap() {
        for addr in u16::MIN..=u16::MAX {
            let hits = [
                is_rom_bank0(addr),
                is_rom_switchable(addr),
                is_vram(addr),
                is_sram_window(addr),
                matches!(classify(addr), MemoryRegion::Wram0),
                matches!(classify(addr), MemoryRegion::WramX),
                matches!(classify(addr), MemoryRegion::EchoRam),
                is_oam(addr),
                matches!(classify(addr), MemoryRegion::Unmapped),
                is_io(addr),
                is_hram(addr),
                matches!(classify(addr), MemoryRegion::InterruptEnable),
            ]
            .into_iter()
            .filter(|hit| *hit)
            .count();
            assert_eq!(hits, 1, "predicate overlap at {addr:#06x}");
        }
    }

    #[test]
    fn echo_ram_is_prohibited() {
        assert!(!is_isr_resident_legal_dmg(ECHO_RAM_BASE));
        assert!(!is_isr_resident_legal_dmg(ECHO_RAM_END));
    }

    #[test]
    fn unmapped_is_prohibited() {
        assert!(!is_isr_resident_legal_dmg(UNMAPPED_BASE));
        assert!(!is_isr_resident_legal_dmg(UNMAPPED_END));
    }

    #[test]
    fn ie_byte_is_singleton() {
        assert_eq!(classify(IE_REG), MemoryRegion::InterruptEnable);
        assert!(is_isr_resident_legal_dmg(IE_REG));
        assert!(is_isr_resident_legal_cgb(IE_REG));
        assert!(is_isr_io_register_allowed(IE_REG));
    }

    #[test]
    fn wram_split_dmg_vs_cgb() {
        assert!(is_fixed_wram_dmg(0xD500));
        assert!(!is_fixed_wram_cgb(0xD500));
    }
}
