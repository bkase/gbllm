//! Joypad register constants and post-decode button state.

use serde::{Deserialize, Serialize};

pub const JOYP_REGISTER: u16 = crate::memory::io_register(0x00);
/// Write pattern for selecting the action-button column.
///
/// JOYP select lines are active-low: P15 / bit 5 is cleared, P14 / bit 4 is set.
pub const JOYP_SELECT_BUTTONS: u8 = 0b0001_0000;
/// Write pattern for selecting the direction-button column.
///
/// JOYP select lines are active-low: P14 / bit 4 is cleared, P15 / bit 5 is set.
pub const JOYP_SELECT_DIRECTIONS: u8 = 0b0010_0000;
pub const JOYP_INPUT_MASK: u8 = 0b0000_1111;
pub const JOYP_UNUSED_HIGH_MASK: u8 = 0b1100_0000;

pub const JOYP_BIT_A: u8 = 0b0000_0001;
pub const JOYP_BIT_B: u8 = 0b0000_0010;
pub const JOYP_BIT_SELECT: u8 = 0b0000_0100;
pub const JOYP_BIT_START: u8 = 0b0000_1000;

pub const JOYP_BIT_RIGHT: u8 = 0b0000_0001;
pub const JOYP_BIT_LEFT: u8 = 0b0000_0010;
pub const JOYP_BIT_UP: u8 = 0b0000_0100;
pub const JOYP_BIT_DOWN: u8 = 0b0000_1000;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum Button {
    A = 0,
    B = 1,
    Select = 2,
    Start = 3,
    Up = 4,
    Down = 5,
    Left = 6,
    Right = 7,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum JoypadColumn {
    Buttons,
    Directions,
}

impl Button {
    pub const ALL: [Button; 8] = [
        Button::A,
        Button::B,
        Button::Select,
        Button::Start,
        Button::Up,
        Button::Down,
        Button::Left,
        Button::Right,
    ];

    #[must_use]
    pub const fn state_mask(self) -> u8 {
        1_u8 << (self as u8)
    }

    #[must_use]
    pub const fn column(self) -> JoypadColumn {
        match self {
            Self::A | Self::B | Self::Select | Self::Start => JoypadColumn::Buttons,
            Self::Up | Self::Down | Self::Left | Self::Right => JoypadColumn::Directions,
        }
    }

    #[must_use]
    pub const fn joyp_column_bit(self) -> u8 {
        match self {
            Self::A | Self::Right => JOYP_BIT_A,
            Self::B | Self::Left => JOYP_BIT_B,
            Self::Select | Self::Up => JOYP_BIT_SELECT,
            Self::Start | Self::Down => JOYP_BIT_START,
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Default, Serialize, Deserialize)]
pub struct ButtonState {
    bits: u8,
}

impl ButtonState {
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self { bits }
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }

    #[must_use]
    pub const fn is_pressed(&self, button: Button) -> bool {
        (self.bits & button.state_mask()) != 0
    }

    #[must_use]
    pub const fn just_pressed(prev: Self, cur: Self, button: Button) -> bool {
        cur.is_pressed(button) && !prev.is_pressed(button)
    }

    #[must_use]
    pub const fn just_released(prev: Self, cur: Self, button: Button) -> bool {
        prev.is_pressed(button) && !cur.is_pressed(button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory;

    #[test]
    fn register_address() {
        assert_eq!(JOYP_REGISTER, memory::IO_BASE);
        assert!(memory::is_io(JOYP_REGISTER));
    }

    #[test]
    fn select_bits() {
        assert_eq!(JOYP_SELECT_BUTTONS, 0x10);
        assert_eq!(JOYP_SELECT_DIRECTIONS, 0x20);
        assert_eq!(JOYP_INPUT_MASK, 0x0F);
        assert_eq!(JOYP_UNUSED_HIGH_MASK, 0xC0);
    }

    #[test]
    fn button_enum_exhaustive() {
        assert_eq!(Button::ALL.len(), 8);
        assert_eq!(Button::ALL[0], Button::A);
        assert_eq!(Button::ALL[7], Button::Right);
    }

    #[test]
    fn is_pressed_table_driven() {
        for bits in u8::MIN..=u8::MAX {
            let state = ButtonState::from_bits(bits);
            for button in Button::ALL {
                assert_eq!(
                    state.is_pressed(button),
                    (bits & button.state_mask()) != 0,
                    "bits={bits:#04x}, button={button:?}"
                );
            }
        }
    }

    #[test]
    fn button_column_mapping_matches_joyp_mux() {
        for (button, column, bit) in [
            (Button::A, JoypadColumn::Buttons, JOYP_BIT_A),
            (Button::B, JoypadColumn::Buttons, JOYP_BIT_B),
            (Button::Select, JoypadColumn::Buttons, JOYP_BIT_SELECT),
            (Button::Start, JoypadColumn::Buttons, JOYP_BIT_START),
            (Button::Right, JoypadColumn::Directions, JOYP_BIT_RIGHT),
            (Button::Left, JoypadColumn::Directions, JOYP_BIT_LEFT),
            (Button::Up, JoypadColumn::Directions, JOYP_BIT_UP),
            (Button::Down, JoypadColumn::Directions, JOYP_BIT_DOWN),
        ] {
            assert_eq!(button.column(), column);
            assert_eq!(button.joyp_column_bit(), bit);
        }
    }

    #[test]
    fn just_pressed_edge() {
        let prev = ButtonState::from_bits(0);
        let cur = ButtonState::from_bits(1 << Button::A as u8);
        assert!(ButtonState::just_pressed(prev, cur, Button::A));
        assert!(!ButtonState::just_pressed(cur, cur, Button::A));
        assert!(!ButtonState::just_pressed(cur, prev, Button::A));
    }

    #[test]
    fn just_released_edge() {
        let prev = ButtonState::from_bits(1 << Button::A as u8);
        let cur = ButtonState::from_bits(0);
        assert!(ButtonState::just_released(prev, cur, Button::A));
        assert!(!ButtonState::just_released(prev, prev, Button::A));
        assert!(!ButtonState::just_released(cur, prev, Button::A));
    }

    #[test]
    fn default_state_is_no_buttons() {
        assert_eq!(ButtonState::default().bits(), 0);
    }

    #[test]
    fn serde_round_trip() {
        let state = ButtonState::from_bits(0b1010_0101);
        let encoded = serde_json::to_string(&state).unwrap();
        let decoded: ButtonState = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, state);
    }
}
