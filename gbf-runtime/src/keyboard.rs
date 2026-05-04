//! On-screen keyboard layout and prompt-buffer input step.

use gbf_asm::builder::Builder;
use gbf_asm::isa::{AluSrc8, Cond, DirectAddr, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data};
use gbf_asm::section::{Section, SectionRole, SymbolicBranch};
use gbf_asm::symbols::SymbolName;
use serde::{Deserialize, Serialize};

use crate::joypad::{JOYPAD_CACHED_STATE_ADDR, JOYPAD_PREV_STATE_ADDR};
use crate::{SECTION_ID_KEYBOARD, SECTION_ID_KEYBOARD_LAYOUT, WramAddr};

pub const PROMPT_BUFFER_BASE_ADDR: WramAddr = WramAddr::new(0xC380);
pub const PROMPT_BUFFER_LEN: u8 = 96;
pub const PROMPT_CURSOR_ADDR: WramAddr = WramAddr::new(0xC3E0);
pub const PROMPT_SUBMITTED_FLAG_ADDR: WramAddr = WramAddr::new(0xC3E1);
pub const KEYBOARD_CURSOR_ADDR: WramAddr = WramAddr::new(0xC3E2);
pub const KEYBOARD_WORK_CELL_KIND_ADDR: WramAddr = WramAddr::new(0xC3E3);
pub const KEYBOARD_WORK_CELL_VALUE_ADDR: WramAddr = WramAddr::new(0xC3E4);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardLayoutSpec<'a> {
    pub rows: u8,
    pub columns: u8,
    pub cells: &'a [KeyboardCell],
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyboardLayoutManifest {
    pub rows: u8,
    pub columns: u8,
    pub cells: Vec<KeyboardCell>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KeyboardCell {
    Char(u8),
    Special(SpecialKey),
    Empty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SpecialKey {
    Backspace,
    Submit,
    Shift,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyboardState {
    pub cursor_row: u8,
    pub cursor_col: u8,
    pub charset_slice: u8,
}

impl KeyboardState {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cursor_row: 0,
            cursor_col: 0,
            charset_slice: 0,
        }
    }

    #[must_use]
    pub const fn move_by(
        self,
        row_delta: i8,
        col_delta: i8,
        layout: KeyboardLayoutSpec<'_>,
    ) -> Self {
        let max_row = layout.rows.saturating_sub(1);
        let max_col = layout.columns.saturating_sub(1);
        Self {
            cursor_row: clamp_delta(self.cursor_row, row_delta, max_row),
            cursor_col: clamp_delta(self.cursor_col, col_delta, max_col),
            charset_slice: self.charset_slice,
        }
    }
}

impl Default for KeyboardState {
    fn default() -> Self {
        Self::new()
    }
}

pub const DEFAULT_LAYOUT_CELLS: [KeyboardCell; 40] = [
    KeyboardCell::Char(b'a'),
    KeyboardCell::Char(b'b'),
    KeyboardCell::Char(b'c'),
    KeyboardCell::Char(b'd'),
    KeyboardCell::Char(b'e'),
    KeyboardCell::Char(b'f'),
    KeyboardCell::Char(b'g'),
    KeyboardCell::Char(b'h'),
    KeyboardCell::Char(b'i'),
    KeyboardCell::Char(b'j'),
    KeyboardCell::Char(b'k'),
    KeyboardCell::Char(b'l'),
    KeyboardCell::Char(b'm'),
    KeyboardCell::Char(b'n'),
    KeyboardCell::Char(b'o'),
    KeyboardCell::Char(b'p'),
    KeyboardCell::Char(b'q'),
    KeyboardCell::Char(b'r'),
    KeyboardCell::Char(b's'),
    KeyboardCell::Char(b't'),
    KeyboardCell::Char(b'u'),
    KeyboardCell::Char(b'v'),
    KeyboardCell::Char(b'w'),
    KeyboardCell::Char(b'x'),
    KeyboardCell::Char(b'y'),
    KeyboardCell::Char(b'z'),
    KeyboardCell::Char(b'0'),
    KeyboardCell::Char(b'1'),
    KeyboardCell::Char(b'2'),
    KeyboardCell::Char(b'3'),
    KeyboardCell::Char(b'4'),
    KeyboardCell::Char(b'5'),
    KeyboardCell::Char(b'6'),
    KeyboardCell::Char(b'7'),
    KeyboardCell::Char(b'8'),
    KeyboardCell::Char(b'9'),
    KeyboardCell::Char(b'.'),
    KeyboardCell::Char(b' '),
    KeyboardCell::Special(SpecialKey::Backspace),
    KeyboardCell::Special(SpecialKey::Submit),
];

#[must_use]
pub const fn default_layout() -> KeyboardLayoutSpec<'static> {
    KeyboardLayoutSpec {
        rows: 4,
        columns: 10,
        cells: &DEFAULT_LAYOUT_CELLS,
    }
}

#[must_use]
pub fn default_layout_manifest() -> KeyboardLayoutManifest {
    let layout = default_layout();
    KeyboardLayoutManifest {
        rows: layout.rows,
        columns: layout.columns,
        cells: layout.cells.to_vec(),
    }
}

pub fn build_keyboard_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_KEYBOARD,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("keyboard", "section").expect("static symbol"),
    );
    builder.label(SymbolName::runtime("keyboard", "step").expect("static symbol"));
    emit_keyboard_step(&mut builder);
    builder.emit(Instr::Ret { cond: None });
    builder.finish().with_size_hint_bytes(360)
}

pub fn build_layout_data_section() -> Section {
    let layout = default_layout_manifest();
    let mut bytes = Vec::with_capacity(2 + layout.cells.len() * 2);
    bytes.push(layout.rows);
    bytes.push(layout.columns);
    for cell in layout.cells {
        match cell {
            KeyboardCell::Char(ch) => {
                bytes.push(0);
                bytes.push(ch);
            }
            KeyboardCell::Special(key) => {
                bytes.push(1);
                bytes.push(match key {
                    SpecialKey::Backspace => 0,
                    SpecialKey::Submit => 1,
                    SpecialKey::Shift => 2,
                    SpecialKey::Cancel => 3,
                });
            }
            KeyboardCell::Empty => {
                bytes.push(2);
                bytes.push(0);
            }
        }
    }

    let mut builder = Builder::new_with_id(
        SECTION_ID_KEYBOARD_LAYOUT,
        SectionRole::Bank0Data,
        SymbolName::runtime("keyboard", "layout_data").expect("static symbol"),
    );
    builder.db_bytes(bytes);
    builder.finish()
}

pub fn emit_keyboard_step(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(JOYPAD_CACHED_STATE_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(JOYPAD_PREV_STATE_ADDR.get()),
    });
    b.emit(Instr::Cpl);
    b.emit(Instr::AndA {
        src: AluSrc8::Reg(Reg8::B),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });

    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::Button::Right.state_mask()),
    });
    keyboard_jump(b, "after_right", Some(Cond::Z));
    emit_keyboard_cursor_increment(b);
    b.label(keyboard_label("after_right"));

    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::Button::Left.state_mask()),
    });
    keyboard_jump(b, "after_left", Some(Cond::Z));
    emit_keyboard_cursor_decrement(b);
    b.label(keyboard_label("after_left"));

    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::Button::A.state_mask()),
    });
    keyboard_jump(b, "after_a", Some(Cond::Z));
    emit_load_layout_cell_to_work(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_WORK_CELL_KIND_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    keyboard_jump(b, "accept_char", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(1),
    });
    keyboard_jump(b, "accept_special", Some(Cond::Z));
    keyboard_jump(b, "after_a", None);

    b.label(keyboard_label("accept_char"));
    emit_load_prompt_hl_from_cursor(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_WORK_CELL_VALUE_ADDR.get()),
    });
    b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    emit_load_screen_bc_from_cursor_a(
        b,
        "accept_char_cursor_div_loop",
        "accept_char_cursor_div_done",
    );
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::E,
    });
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("video_commit", "enqueue_glyph_cell").expect("static symbol"),
        None,
    ));
    emit_prompt_cursor_increment(b);
    keyboard_jump(b, "after_a", None);

    b.label(keyboard_label("accept_special"));
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_WORK_CELL_VALUE_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    keyboard_jump(b, "special_backspace", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(1),
    });
    keyboard_jump(b, "special_submit", Some(Cond::Z));
    keyboard_jump(b, "after_a", None);

    b.label(keyboard_label("special_backspace"));
    emit_prompt_cursor_decrement(b);
    keyboard_jump(b, "after_a", None);

    b.label(keyboard_label("special_submit"));
    emit_submit_flag_set(b);
    b.label(keyboard_label("after_a"));

    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(gbf_hw::joypad::Button::Start.state_mask()),
    });
    keyboard_jump(b, "after_start", Some(Cond::Z));
    emit_submit_flag_set(b);
    b.label(keyboard_label("after_start"));
}

#[must_use]
pub fn apply_special_key(cursor: u8, submitted: bool, key: SpecialKey) -> (u8, bool) {
    match key {
        SpecialKey::Backspace => (cursor.saturating_sub(1), submitted),
        SpecialKey::Submit => (cursor, true),
        SpecialKey::Shift | SpecialKey::Cancel => (cursor, submitted),
    }
}

const fn clamp_delta(value: u8, delta: i8, max: u8) -> u8 {
    if delta < 0 {
        value.saturating_sub(delta.unsigned_abs())
    } else {
        let next = value.saturating_add(delta as u8);
        if next > max { max } else { next }
    }
}

fn emit_prompt_cursor_increment(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(PROMPT_BUFFER_LEN),
    });
    keyboard_jump(b, "prompt_cursor_increment_store", Some(Cond::C));
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: PROMPT_BUFFER_LEN - 1,
    });
    b.label(keyboard_label("prompt_cursor_increment_store"));
    b.emit(Instr::LdDirectFromA {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
}

fn emit_prompt_cursor_decrement(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    keyboard_jump(b, "cursor_decrement_done", Some(Cond::Z));
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: b' ',
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    emit_load_prompt_hl_from_cursor(b);
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::E,
    });
    b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
    b.emit(Instr::LdAFromDirect {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    emit_load_screen_bc_from_cursor_a(b, "backspace_cursor_div_loop", "backspace_cursor_div_done");
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::E,
    });
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("video_commit", "enqueue_glyph_cell").expect("static symbol"),
        None,
    ));
    b.label(keyboard_label("cursor_decrement_done"));
}

fn emit_load_screen_bc_from_cursor_a(
    b: &mut Builder,
    loop_label: &'static str,
    done_label: &'static str,
) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::C,
        imm: 0,
    });
    b.label(keyboard_label(loop_label));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(20),
    });
    keyboard_jump(b, done_label, Some(Cond::C));
    b.emit(Instr::SubA {
        src: AluSrc8::Imm(20),
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    keyboard_jump(b, loop_label, None);
    b.label(keyboard_label(done_label));
    b.emit(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
}

fn emit_keyboard_cursor_increment(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_CURSOR_ADDR.get()),
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(DEFAULT_LAYOUT_CELLS.len() as u8),
    });
    keyboard_jump(b, "keyboard_cursor_increment_store", Some(Cond::C));
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: DEFAULT_LAYOUT_CELLS.len() as u8 - 1,
    });
    b.label(keyboard_label("keyboard_cursor_increment_store"));
    b.emit(Instr::LdDirectFromA {
        addr: direct(KEYBOARD_CURSOR_ADDR.get()),
    });
}

fn emit_keyboard_cursor_decrement(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_CURSOR_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    keyboard_jump(b, "keyboard_cursor_decrement_done", Some(Cond::Z));
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(KEYBOARD_CURSOR_ADDR.get()),
    });
    b.label(keyboard_label("keyboard_cursor_decrement_done"));
}

fn emit_load_layout_cell_to_work(b: &mut Builder) {
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: crate::KEYBOARD_LAYOUT_DATA_ADDR + 2,
    });
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::DE,
        imm: 2,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(KEYBOARD_CURSOR_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::Z),
        off: 4,
    });
    b.label(keyboard_label("layout_cell_loop"));
    b.emit(Instr::AddHl { src: Reg16Data::DE });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::NZ),
        off: -4,
    });
    b.emit(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(KEYBOARD_WORK_CELL_KIND_ADDR.get()),
    });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(KEYBOARD_WORK_CELL_VALUE_ADDR.get()),
    });
}

fn emit_submit_flag_set(b: &mut Builder) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 1,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(PROMPT_SUBMITTED_FLAG_ADDR.get()),
    });
}

fn emit_load_prompt_hl_from_cursor(b: &mut Builder) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::H,
        imm: (PROMPT_BUFFER_BASE_ADDR.get() >> 8) as u8,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(PROMPT_CURSOR_ADDR.get()),
    });
    b.emit(Instr::AddA {
        src: AluSrc8::Imm((PROMPT_BUFFER_BASE_ADDR.get() & 0x00FF) as u8),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
}

fn keyboard_jump(b: &mut Builder, target: &'static str, cond: Option<Cond>) {
    b.branch(SymbolicBranch::jump(keyboard_label(target), cond));
}

fn keyboard_label(target: &'static str) -> SymbolName {
    SymbolName::runtime("keyboard_step", target).expect("static symbol")
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("keyboard WRAM address is below high memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section_effect_kinds;
    use gbf_asm::effect::MachineEffectKind;

    #[test]
    fn default_layout() {
        let layout = super::default_layout();
        assert_eq!(layout.rows, 4);
        assert_eq!(layout.columns, 10);
        assert_eq!(layout.cells.len(), 40);
        assert_eq!(layout.cells[0], KeyboardCell::Char(b'a'));
        assert_eq!(layout.cells[25], KeyboardCell::Char(b'z'));
        assert_eq!(
            layout.cells[38],
            KeyboardCell::Special(SpecialKey::Backspace)
        );
        assert_eq!(layout.cells[39], KeyboardCell::Special(SpecialKey::Submit));
        assert!(
            !layout
                .cells
                .contains(&KeyboardCell::Special(SpecialKey::Shift))
        );
        assert!(
            !layout
                .cells
                .contains(&KeyboardCell::Special(SpecialKey::Cancel))
        );
    }

    #[test]
    fn cursor_movement() {
        let layout = super::default_layout();
        let state = KeyboardState::new();
        assert_eq!(state.move_by(-1, -1, layout), state);
        let state = state.move_by(3, 9, layout);
        assert_eq!(state.cursor_row, 3);
        assert_eq!(state.cursor_col, 9);
        assert_eq!(state.move_by(1, 1, layout), state);
    }

    #[test]
    fn special_keys() {
        assert_eq!(
            apply_special_key(3, false, SpecialKey::Backspace),
            (2, false)
        );
        assert_eq!(
            apply_special_key(0, false, SpecialKey::Backspace),
            (0, false)
        );
        assert_eq!(apply_special_key(3, false, SpecialKey::Submit), (3, true));
    }

    #[test]
    fn step_emits_only_queue_ops() {
        let section = build_keyboard_section();
        assert!(!section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
        assert!(!section_effect_kinds(&section).contains(&MachineEffectKind::StoreToOam));
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToWram));
    }

    #[test]
    fn step_uses_layout_cell_value_for_a_press() {
        let section = build_keyboard_section();
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::Ld16Imm {
                    imm,
                    ..
                } if imm == crate::KEYBOARD_LAYOUT_DATA_ADDR + 2
            )
        }));
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::LdDirectFromA { addr } if addr.get() == KEYBOARD_WORK_CELL_VALUE_ADDR.get()
            )
        }));
        assert!(
            !section
                .instrs()
                .iter()
                .any(|item| { matches!(item.data, Instr::Ld8RegFromImm { imm: b'a', .. }) })
        );
    }

    #[test]
    fn prompt_buffer_addresses_valid() {
        let start = PROMPT_BUFFER_BASE_ADDR.get();
        let end = start + u16::from(PROMPT_BUFFER_LEN);
        assert!(gbf_hw::memory::is_wram(start));
        assert!(gbf_hw::memory::is_wram(end - 1));
        assert!(end <= PROMPT_CURSOR_ADDR.get());
        assert_eq!(
            PROMPT_CURSOR_ADDR.get() + 1,
            PROMPT_SUBMITTED_FLAG_ADDR.get()
        );
        assert_eq!(
            PROMPT_SUBMITTED_FLAG_ADDR.get() + 1,
            KEYBOARD_CURSOR_ADDR.get()
        );
    }

    #[test]
    fn keyboard_never_reads_joyp_directly() {
        let section = build_keyboard_section();
        assert!(!section.instrs().iter().any(|item| {
            matches!(item.data, Instr::LdAFromHighDirect { offset } if offset.absolute_addr() == gbf_hw::joypad::JOYP_REGISTER)
        }));
    }
}
