use gbf_hw::interrupts::{
    DIV_REGISTER, IE_REGISTER, IF_REGISTER, INT_VECTOR_TIMER, InterruptSource,
    TAC_CLOCK_SELECT_MASK, TAC_ENABLE_BIT, TAC_REGISTER, TIMA_REGISTER, TMA_REGISTER, bit_mask,
    vector_for,
};
use gbf_hw::joypad::{JOYP_INPUT_MASK, JOYP_REGISTER, JOYP_SELECT_BUTTONS, JOYP_SELECT_DIRECTIONS};
use gbf_hw::lcd::{
    BGP_REG, DMA_REG, LCDC_REG, LY_REG, LYC_REG, OBP0_REG, OBP1_REG, SCX_REG, SCY_REG, STAT_REG,
    WX_REG, WY_REG,
};
use gbf_hw::mbc5::{
    MBC5_BANK1_BASE, MBC5_BANK1_END, MBC5_BANK2_BASE, MBC5_BANK2_END, MBC5_RAMB_BASE,
    MBC5_RAMB_END, MBC5_RAMG_BASE, MBC5_RAMG_END, MBC5_RESERVED_BASE, MBC5_RESERVED_END,
    MbcRegisterClass, classify_mbc_write_address,
};
use gbf_hw::memory;
use gbf_hw::target::{BRINGUP_TARGET_PROFILE_ID, dmg_mbc5_8mib_128kib};
use gbf_hw::timing::{FRAME_M_CYCLES, VBLANK_M_CYCLES};

#[test]
fn interrupt_vectors_live_in_bank0() {
    for source in InterruptSource::ALL {
        assert!(memory::is_rom_bank0(vector_for(source)));
        assert_eq!(bit_mask(source), 1_u8 << (source as u8));
    }
    assert_eq!(INT_VECTOR_TIMER, 0x0050);
}

#[test]
fn io_registers_live_in_io_or_ie_regions() {
    for register in [
        IF_REGISTER,
        DIV_REGISTER,
        TIMA_REGISTER,
        TMA_REGISTER,
        TAC_REGISTER,
        LCDC_REG,
        STAT_REG,
        SCY_REG,
        SCX_REG,
        LY_REG,
        LYC_REG,
        DMA_REG,
        BGP_REG,
        OBP0_REG,
        OBP1_REG,
        WY_REG,
        WX_REG,
        JOYP_REGISTER,
    ] {
        assert!(
            memory::is_io(register),
            "register should live in IO: {register:#06x}"
        );
    }
    assert_eq!(IE_REGISTER, memory::IE_REG);
    assert_eq!(
        memory::classify(IE_REGISTER),
        memory::MemoryRegion::InterruptEnable
    );
    assert_eq!(JOYP_SELECT_BUTTONS, 0x10);
    assert_eq!(JOYP_SELECT_DIRECTIONS, 0x20);
    assert_eq!(JOYP_INPUT_MASK, 0x0F);
    assert_eq!(TAC_ENABLE_BIT, 0x04);
    assert_eq!(TAC_CLOCK_SELECT_MASK, 0x03);
}

#[test]
fn mbc5_bands_live_in_cartridge_rom_address_space() {
    assert_eq!(
        classify_mbc_write_address(MBC5_RAMG_BASE),
        Some(MbcRegisterClass::Ramg)
    );
    assert_eq!(
        classify_mbc_write_address(MBC5_BANK1_BASE),
        Some(MbcRegisterClass::Bank1)
    );
    assert_eq!(
        classify_mbc_write_address(MBC5_BANK2_BASE),
        Some(MbcRegisterClass::Bank2)
    );
    assert_eq!(
        classify_mbc_write_address(MBC5_RAMB_BASE),
        Some(MbcRegisterClass::Ramb)
    );
    for address in [MBC5_RAMG_BASE, MBC5_BANK1_BASE, MBC5_BANK2_BASE] {
        assert!(memory::is_rom_bank0(address));
    }
    assert!(memory::is_rom_switchable(MBC5_RAMB_BASE));
    assert!(memory::is_rom_switchable(MBC5_RESERVED_BASE));
    assert!(memory::is_rom_switchable(MBC5_RESERVED_END));
    assert_eq!(MBC5_RAMG_END + 1, MBC5_BANK1_BASE);
    assert_eq!(MBC5_BANK1_END + 1, MBC5_BANK2_BASE);
    assert_eq!(MBC5_BANK2_END + 1, MBC5_RAMB_BASE);
    assert_eq!(MBC5_RAMB_END + 1, MBC5_RESERVED_BASE);
}

#[test]
fn bring_up_profile_smoke() {
    let profile = dmg_mbc5_8mib_128kib();
    assert_eq!(profile.id().as_str(), BRINGUP_TARGET_PROFILE_ID);
    assert_eq!(profile.timing().frame_m_cycles(), FRAME_M_CYCLES);
    assert_eq!(profile.timing().vblank_m_cycles(), VBLANK_M_CYCLES);
    assert_eq!(profile.cartridge().rom_size().bank_count(), 512);
    assert_eq!(profile.cartridge().ram_size().bank_count(), 16);
}
