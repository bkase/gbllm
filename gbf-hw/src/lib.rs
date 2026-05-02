//! Hardware target contracts, memory maps, timing models, calibration schema, and Game Boy constants.

#![forbid(unsafe_code)]

pub mod calibration;
pub mod cartridge_header;
pub mod interrupts;
pub mod joypad;
pub mod lcd;
pub mod mbc5;
pub mod memory;
pub mod target;
pub mod timing;
