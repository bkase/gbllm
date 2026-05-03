//! Text layout, M0 font asset, and queue-staged glyph updates.

use gbf_asm::builder::Builder;
use gbf_asm::isa::Instr;
use gbf_asm::section::{Section, SectionRole};
use gbf_asm::symbols::SymbolName;
use serde::{Deserialize, Serialize};

use crate::video_commit::{self, UiCommitOp};
use crate::{SECTION_ID_TEXT, SECTION_ID_TEXT_FONT};

pub const FONT_TILE_COUNT: u16 = 128;
pub const FONT_BYTES_PER_TILE: usize = 16;
pub const FONT_BYTES_LEN: usize = FONT_TILE_COUNT as usize * FONT_BYTES_PER_TILE;

static FONT_BYTES: &[u8; FONT_BYTES_LEN] = include_bytes!("../assets/font_8x8.bin");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TextLayout {
    pub bg_map_origin: u16,
    pub visible_columns: u8,
    pub visible_rows: u8,
    pub bg_map_stride: u8,
}

impl TextLayout {
    #[must_use]
    pub const fn dmg_default() -> Self {
        Self {
            bg_map_origin: 0x9800,
            visible_columns: 20,
            visible_rows: 18,
            bg_map_stride: 32,
        }
    }

    #[must_use]
    pub const fn cell_addr(self, x: u8, y: u8) -> u16 {
        self.bg_map_origin + y as u16 * self.bg_map_stride as u16 + x as u16
    }
}

pub fn build_text_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_TEXT,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("text", "section").expect("static symbol"),
    );
    builder.label(SymbolName::runtime("text", "print_glyph").expect("static symbol"));
    emit_text_print_glyph(&mut builder, 0, 0, 0);
    builder.emit(Instr::Ret { cond: None });
    builder.label(SymbolName::runtime("text", "clear_row").expect("static symbol"));
    emit_text_clear_row(&mut builder, 0);
    builder.emit(Instr::Ret { cond: None });
    builder.finish().with_size_hint_bytes(192)
}

pub fn build_font_data_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_TEXT_FONT,
        SectionRole::Bank0Data,
        SymbolName::runtime("text", "font_data").expect("static symbol"),
    );
    builder.db_bytes(font_bytes());
    builder.finish()
}

#[must_use]
pub fn font_bytes() -> &'static [u8] {
    FONT_BYTES
}

pub fn emit_text_print_glyph(b: &mut Builder, x: u8, y: u8, glyph: u8) {
    video_commit::emit_queue_op(b, UiCommitOp::PutGlyphCell { x, y, glyph });
}

pub fn emit_text_clear_row(b: &mut Builder, y: u8) {
    video_commit::emit_queue_op(
        b,
        UiCommitOp::FillGlyphRun {
            x: 0,
            y,
            len: TextLayout::dmg_default().visible_columns,
            glyph: b' ',
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section_effect_kinds;
    use gbf_asm::effect::MachineEffectKind;

    #[test]
    fn font_size() {
        assert_eq!(font_bytes().len(), FONT_BYTES_LEN);
        assert_eq!(font_bytes().len(), FONT_TILE_COUNT as usize * 16);
    }

    #[test]
    fn layout_dmg() {
        let layout = TextLayout::dmg_default();
        assert_eq!(layout.visible_columns, 20);
        assert_eq!(layout.visible_rows, 18);
        assert_eq!(layout.bg_map_stride, 32);
        assert_eq!(layout.cell_addr(1, 1), 0x9800 + 32 + 1);
    }

    #[test]
    fn print_glyph_stages() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "text").unwrap(),
        );
        emit_text_print_glyph(&mut builder, 2, 3, 4);
        let section = builder.finish();
        let immediates: Vec<u8> = section
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::Ld8RegFromImm { imm, .. } => Some(imm),
                Instr::Ld8HlFromImm { imm } => Some(imm),
                _ => None,
            })
            .collect();
        assert!(immediates.contains(&(video_commit::UiCommitOpKind::PutGlyphCell as u8)));
        assert!(immediates.contains(&2));
        assert!(immediates.contains(&3));
        assert!(immediates.contains(&4));
    }

    #[test]
    fn no_vram_access_machine_effect() {
        let section = build_text_section();
        assert!(!section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
        assert!(!section_effect_kinds(&section).contains(&MachineEffectKind::StoreToOam));
    }

    #[test]
    fn font_installed_before_lcdc_enable() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "bootstrap").unwrap(),
        );
        video_commit::emit_bootstrap_vram_init(&mut builder);
        let section = builder.finish();
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::LoadFromIo));
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
    }
}
