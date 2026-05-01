//! LR35902 interrupt vectors, IE/IF bits, and timer register constants.

use serde::{Deserialize, Serialize};

pub const INT_VECTOR_VBLANK: u16 = 0x0040;
pub const INT_VECTOR_LCD_STAT: u16 = 0x0048;
pub const INT_VECTOR_TIMER: u16 = 0x0050;
pub const INT_VECTOR_SERIAL: u16 = 0x0058;
pub const INT_VECTOR_JOYPAD: u16 = 0x0060;

pub use crate::memory::IE_REG as IE_REGISTER;
pub const IF_REGISTER: u16 = crate::memory::io_register(0x0F);

pub const DIV_REGISTER: u16 = crate::memory::io_register(0x04);
pub const TIMA_REGISTER: u16 = crate::memory::io_register(0x05);
pub const TMA_REGISTER: u16 = crate::memory::io_register(0x06);
pub const TAC_REGISTER: u16 = crate::memory::io_register(0x07);
pub const TAC_ENABLE_BIT: u8 = 0b0000_0100;
pub const TAC_CLOCK_SELECT_MASK: u8 = 0b0000_0011;
pub const IE_IF_UNUSED_MASK: u8 = 0b1110_0000;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum InterruptSource {
    VBlank = 0,
    LcdStat = 1,
    Timer = 2,
    Serial = 3,
    Joypad = 4,
}

impl InterruptSource {
    pub const ALL: [InterruptSource; 5] = [
        InterruptSource::VBlank,
        InterruptSource::LcdStat,
        InterruptSource::Timer,
        InterruptSource::Serial,
        InterruptSource::Joypad,
    ];
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TacClockSelect {
    Hz4096 = 0b00,
    Hz262144 = 0b01,
    Hz65536 = 0b10,
    Hz16384 = 0b11,
}

impl TacClockSelect {
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        match bits & TAC_CLOCK_SELECT_MASK {
            0b00 => Self::Hz4096,
            0b01 => Self::Hz262144,
            0b10 => Self::Hz65536,
            _ => Self::Hz16384,
        }
    }

    #[must_use]
    pub const fn frequency_hz(self) -> u32 {
        match self {
            Self::Hz4096 => 4_096,
            Self::Hz262144 => 262_144,
            Self::Hz65536 => 65_536,
            Self::Hz16384 => 16_384,
        }
    }
}

#[must_use]
pub const fn vector_for(source: InterruptSource) -> u16 {
    match source {
        InterruptSource::VBlank => INT_VECTOR_VBLANK,
        InterruptSource::LcdStat => INT_VECTOR_LCD_STAT,
        InterruptSource::Timer => INT_VECTOR_TIMER,
        InterruptSource::Serial => INT_VECTOR_SERIAL,
        InterruptSource::Joypad => INT_VECTOR_JOYPAD,
    }
}

#[must_use]
pub const fn bit_mask(source: InterruptSource) -> u8 {
    1_u8 << (source as u8)
}

#[must_use]
pub const fn ie_bit(source: InterruptSource) -> u8 {
    bit_mask(source)
}

#[must_use]
pub const fn if_bit(source: InterruptSource) -> u8 {
    bit_mask(source)
}

#[must_use]
pub const fn highest_pending(ie: u8, if_: u8) -> Option<InterruptSource> {
    let pending = ie & if_;
    if pending & bit_mask(InterruptSource::VBlank) != 0 {
        Some(InterruptSource::VBlank)
    } else if pending & bit_mask(InterruptSource::LcdStat) != 0 {
        Some(InterruptSource::LcdStat)
    } else if pending & bit_mask(InterruptSource::Timer) != 0 {
        Some(InterruptSource::Timer)
    } else if pending & bit_mask(InterruptSource::Serial) != 0 {
        Some(InterruptSource::Serial)
    } else if pending & bit_mask(InterruptSource::Joypad) != 0 {
        Some(InterruptSource::Joypad)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory;

    #[test]
    fn vector_table() {
        assert_eq!(vector_for(InterruptSource::VBlank), 0x0040);
        assert_eq!(vector_for(InterruptSource::LcdStat), 0x0048);
        assert_eq!(vector_for(InterruptSource::Timer), 0x0050);
        assert_eq!(vector_for(InterruptSource::Serial), 0x0058);
        assert_eq!(vector_for(InterruptSource::Joypad), 0x0060);
    }

    #[test]
    fn ie_if_bit_layout() {
        for (source, mask) in [
            (InterruptSource::VBlank, 0x01),
            (InterruptSource::LcdStat, 0x02),
            (InterruptSource::Timer, 0x04),
            (InterruptSource::Serial, 0x08),
            (InterruptSource::Joypad, 0x10),
        ] {
            assert_eq!(bit_mask(source), mask);
            assert_eq!(ie_bit(source), mask);
            assert_eq!(if_bit(source), mask);
        }
    }

    #[test]
    fn priority_order() {
        assert_eq!(
            InterruptSource::ALL,
            [
                InterruptSource::VBlank,
                InterruptSource::LcdStat,
                InterruptSource::Timer,
                InterruptSource::Serial,
                InterruptSource::Joypad,
            ]
        );
        assert!(InterruptSource::VBlank < InterruptSource::LcdStat);
        assert!(InterruptSource::LcdStat < InterruptSource::Timer);
        assert!(InterruptSource::Timer < InterruptSource::Serial);
        assert!(InterruptSource::Serial < InterruptSource::Joypad);
    }

    #[test]
    fn timer_registers_in_io_region() {
        for reg in [DIV_REGISTER, TIMA_REGISTER, TMA_REGISTER, TAC_REGISTER] {
            assert!(memory::is_io(reg));
        }
    }

    #[test]
    fn register_addresses_are_exact() {
        assert_eq!(IF_REGISTER, 0xFF0F);
        assert_eq!(DIV_REGISTER, 0xFF04);
        assert_eq!(TIMA_REGISTER, 0xFF05);
        assert_eq!(TMA_REGISTER, 0xFF06);
        assert_eq!(TAC_REGISTER, 0xFF07);
    }

    #[test]
    fn ie_register_is_singleton() {
        assert_eq!(IE_REGISTER, memory::IE_REG);
        assert_eq!(IE_REGISTER, 0xFFFF);
    }

    #[test]
    fn vectors_in_bank0() {
        for source in InterruptSource::ALL {
            assert!(memory::is_rom_bank0(vector_for(source)));
        }
    }

    #[test]
    fn tac_enable_bit_is_bit_2() {
        assert_eq!(TAC_ENABLE_BIT, 0b0000_0100);
        assert_eq!(TAC_CLOCK_SELECT_MASK, 0b0000_0011);
        assert_eq!(IE_IF_UNUSED_MASK, 0b1110_0000);
    }

    #[test]
    fn tac_clock_select_frequencies() {
        for (bits, select, hz) in [
            (0b00, TacClockSelect::Hz4096, 4_096),
            (0b01, TacClockSelect::Hz262144, 262_144),
            (0b10, TacClockSelect::Hz65536, 65_536),
            (0b11, TacClockSelect::Hz16384, 16_384),
        ] {
            assert_eq!(TacClockSelect::from_bits(bits), select);
            assert_eq!(TacClockSelect::from_bits(bits | 0b1111_1100), select);
            assert_eq!(select.frequency_hz(), hz);
        }
    }

    #[test]
    fn highest_pending_follows_interrupt_priority() {
        let all_pending = InterruptSource::ALL
            .iter()
            .copied()
            .fold(0_u8, |bits, source| bits | bit_mask(source));

        assert_eq!(
            highest_pending(all_pending, all_pending),
            Some(InterruptSource::VBlank)
        );
        assert_eq!(
            highest_pending(
                bit_mask(InterruptSource::Timer) | bit_mask(InterruptSource::Joypad),
                all_pending,
            ),
            Some(InterruptSource::Timer)
        );
        assert_eq!(highest_pending(0, all_pending), None);
    }
}
