//! LCD-mode-gated UI commit queue and the sole VRAM/OAM writer.

use gbf_abi::FaultCode;
use gbf_asm::builder::Builder;
use gbf_asm::isa::{
    AluSrc8, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data,
};
use gbf_asm::section::{ExecutionContext, Section, SectionPrivilege, SectionRole, SymbolicBranch};
use gbf_asm::symbols::SymbolName;
use gbf_hw::lcd::PpuMode;
use serde::{Deserialize, Serialize};
use static_assertions::{const_assert, const_assert_eq};

use crate::{SECTION_ID_VIDEO_COMMIT, WramAddr};

pub const COMMIT_QUEUE_BASE_ADDR: WramAddr = WramAddr::new(0xC200);
pub const COMMIT_QUEUE_LEN: u8 = 32;
pub const UI_COMMIT_WIRE_OP_BYTES: u8 = 8;
pub const MAX_FILL_GLYPH_RUN_CELLS: u8 = 20;
pub const COMMIT_QUEUE_HEAD_ADDR: WramAddr =
    COMMIT_QUEUE_BASE_ADDR.add(COMMIT_QUEUE_LEN as u16 * UI_COMMIT_WIRE_OP_BYTES as u16);
pub const COMMIT_QUEUE_TAIL_ADDR: WramAddr = COMMIT_QUEUE_HEAD_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_X_ADDR: WramAddr = COMMIT_QUEUE_TAIL_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_Y_ADDR: WramAddr = COMMIT_QUEUE_WORK_X_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_LEN_ADDR: WramAddr = COMMIT_QUEUE_WORK_Y_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_GLYPH_ADDR: WramAddr = COMMIT_QUEUE_WORK_LEN_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_TARGET_ADDR: WramAddr = COMMIT_QUEUE_WORK_GLYPH_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_VALUE_ADDR: WramAddr = COMMIT_QUEUE_WORK_TARGET_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_SPRITE_INDEX_ADDR: WramAddr = COMMIT_QUEUE_WORK_VALUE_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_ATTRS_ADDR: WramAddr = COMMIT_QUEUE_WORK_SPRITE_INDEX_ADDR.add(1);
pub const COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR: WramAddr = COMMIT_QUEUE_WORK_ATTRS_ADDR.add(1);
pub const BOOTSTRAP_BG_MAP_ORIGIN: u16 = 0x9800;
pub const BOOTSTRAP_BG_MAP_BYTES: u16 = 32 * 32;

const HBLANK_PREFIX: &str = "video_commit_hblank";
const VBLANK_PREFIX: &str = "video_commit_vblank";
const BOOTSTRAP_PREFIX: &str = "video_commit_bootstrap";

const_assert_eq!(core::mem::size_of::<UiCommitWireOp>(), 8);
const_assert!(COMMIT_QUEUE_BASE_ADDR.get() & 0x00FF == 0);
const_assert!(
    COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR.get() < crate::panic::WRAM_LAST_FAULT_ADDR.get()
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UiCommitOp {
    PutGlyphCell {
        x: u8,
        y: u8,
        glyph: u8,
    },
    FillGlyphRun {
        x: u8,
        y: u8,
        len: u8,
        glyph: u8,
    },
    SetDmgPalette {
        target: DmgPaletteRegister,
        value: u8,
    },
    PutOamSprite {
        sprite_index: u8,
        y: u8,
        x: u8,
        tile: u8,
        attrs: u8,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum UiCommitOpKind {
    PutGlyphCell = 1,
    FillGlyphRun = 2,
    SetDmgPalette = 3,
    PutOamSprite = 4,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DmgPaletteRegister {
    Bgp,
    Obp0,
    Obp1,
}

impl DmgPaletteRegister {
    #[must_use]
    pub const fn addr(self) -> u16 {
        match self {
            Self::Bgp => gbf_hw::lcd::BGP_REG,
            Self::Obp0 => gbf_hw::lcd::OBP0_REG,
            Self::Obp1 => gbf_hw::lcd::OBP1_REG,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UiCommitPlan {
    pub max_ops_per_frame: u16,
    pub max_ops_per_hblank: u8,
    pub vblank_priority_ops: Vec<UiCommitOpKind>,
}

impl UiCommitPlan {
    #[must_use]
    pub fn default_v1() -> Self {
        Self {
            max_ops_per_frame: 32,
            max_ops_per_hblank: 1,
            vblank_priority_ops: vec![UiCommitOpKind::PutOamSprite, UiCommitOpKind::SetDmgPalette],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UiCommitWireOp([u8; UI_COMMIT_WIRE_OP_BYTES as usize]);

impl UiCommitWireOp {
    #[must_use]
    pub const fn bytes(self) -> [u8; UI_COMMIT_WIRE_OP_BYTES as usize] {
        self.0
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; UI_COMMIT_WIRE_OP_BYTES as usize] {
        &self.0
    }

    #[must_use]
    pub const fn encode(op: UiCommitOp) -> Self {
        let mut out = [0_u8; UI_COMMIT_WIRE_OP_BYTES as usize];
        match op {
            UiCommitOp::PutGlyphCell { x, y, glyph } => {
                out[0] = UiCommitOpKind::PutGlyphCell as u8;
                out[2] = x;
                out[3] = y;
                out[4] = glyph;
            }
            UiCommitOp::FillGlyphRun { x, y, len, glyph } => {
                out[0] = UiCommitOpKind::FillGlyphRun as u8;
                out[2] = x;
                out[3] = y;
                out[4] = bounded_fill_run_len(len);
                out[5] = glyph;
            }
            UiCommitOp::SetDmgPalette { target, value } => {
                out[0] = UiCommitOpKind::SetDmgPalette as u8;
                out[2] = match target {
                    DmgPaletteRegister::Bgp => 0,
                    DmgPaletteRegister::Obp0 => 1,
                    DmgPaletteRegister::Obp1 => 2,
                };
                out[3] = value;
            }
            UiCommitOp::PutOamSprite {
                sprite_index,
                y,
                x,
                tile,
                attrs,
            } => {
                out[0] = UiCommitOpKind::PutOamSprite as u8;
                out[2] = sprite_index;
                out[3] = y;
                out[4] = x;
                out[5] = tile;
                out[6] = attrs;
            }
        }
        Self(out)
    }
}

#[must_use]
pub const fn op_kind(op: UiCommitOp) -> UiCommitOpKind {
    match op {
        UiCommitOp::PutGlyphCell { .. } => UiCommitOpKind::PutGlyphCell,
        UiCommitOp::FillGlyphRun { .. } => UiCommitOpKind::FillGlyphRun,
        UiCommitOp::SetDmgPalette { .. } => UiCommitOpKind::SetDmgPalette,
        UiCommitOp::PutOamSprite { .. } => UiCommitOpKind::PutOamSprite,
    }
}

#[must_use]
pub const fn vram_op_legal_in(mode: PpuMode) -> bool {
    gbf_hw::lcd::vram_accessible_in(mode)
}

#[must_use]
pub const fn bounded_fill_run_len(len: u8) -> u8 {
    if len > MAX_FILL_GLYPH_RUN_CELLS {
        MAX_FILL_GLYPH_RUN_CELLS
    } else {
        len
    }
}

#[must_use]
pub const fn oam_op_legal_in(mode: PpuMode) -> bool {
    gbf_hw::lcd::oam_accessible_in(mode)
}

#[must_use]
pub const fn op_legal_in_hblank_drain(kind: UiCommitOpKind, mode: PpuMode) -> bool {
    match kind {
        UiCommitOpKind::PutGlyphCell | UiCommitOpKind::FillGlyphRun => {
            matches!(mode, PpuMode::HBlank | PpuMode::OamSearch)
        }
        UiCommitOpKind::SetDmgPalette => true,
        UiCommitOpKind::PutOamSprite => false,
    }
}

#[must_use]
pub const fn op_legal_in_vblank_drain(kind: UiCommitOpKind, mode: PpuMode) -> bool {
    match kind {
        UiCommitOpKind::PutGlyphCell | UiCommitOpKind::FillGlyphRun => vram_op_legal_in(mode),
        UiCommitOpKind::SetDmgPalette => true,
        UiCommitOpKind::PutOamSprite => oam_op_legal_in(mode),
    }
}

#[must_use]
pub const fn queue_full_fault() -> FaultCode {
    FaultCode::UiCommitQueueFull
}

#[must_use]
pub const fn illegal_mode_fault() -> FaultCode {
    FaultCode::UiCommitOutsideLegalMode
}

pub fn build_video_commit_section() -> Section {
    let mut builder = Builder::new_with_id(
        SECTION_ID_VIDEO_COMMIT,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("video_commit", "section").expect("static symbol"),
    )
    .with_section_privilege(
        SectionPrivilege::normal().with_execution_context(ExecutionContext::VideoCommitOnly),
    );

    builder.label(SymbolName::runtime("video_commit", "enqueue").expect("static symbol"));
    builder
        .label(SymbolName::runtime("video_commit", "enqueue_glyph_cell").expect("static symbol"));
    emit_queue_glyph_cell_from_regs(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("video_commit", "drain_hblank").expect("static symbol"));
    emit_commit_drain_hblank(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.label(SymbolName::runtime("video_commit", "drain_vblank").expect("static symbol"));
    emit_commit_drain_vblank(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder
        .label(SymbolName::runtime("video_commit", "bootstrap_vram_init").expect("static symbol"));
    emit_bootstrap_vram_init(&mut builder);
    builder.emit(Instr::Ret { cond: None });

    builder.finish().with_size_hint_bytes(1100)
}

/// Stage a queue op. The emitted shape writes all payload bytes before the tail
/// byte, preserving the ISR-consumer publication order.
pub fn emit_queue_op(b: &mut Builder, op: UiCommitOp) {
    let wire = UiCommitWireOp::encode(op).bytes();
    emit_compute_next_tail_or_fault(b);

    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    emit_load_slot_hl_from_index_in_a(b);
    for (idx, byte) in wire.into_iter().enumerate() {
        b.emit(Instr::Ld8HlFromImm { imm: byte });
        if idx + 1 != usize::from(UI_COMMIT_WIRE_OP_BYTES) {
            b.emit(Instr::Inc16 { dst: Reg16Data::HL });
        }
    }
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::C,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_TAIL_ADDR.get()),
    });
}

/// Runtime subroutine ABI: `B = x`, `C = y`, `A = glyph`.
pub fn emit_queue_glyph_cell_from_regs(b: &mut Builder) {
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_X_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::C,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_Y_ADDR.get()),
    });

    emit_compute_next_tail_or_fault(b);

    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    emit_load_slot_hl_from_index_in_a(b);
    b.emit(Instr::Ld8HlFromImm {
        imm: UiCommitOpKind::PutGlyphCell as u8,
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8HlFromImm { imm: 0 });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    for addr in [
        COMMIT_QUEUE_WORK_X_ADDR,
        COMMIT_QUEUE_WORK_Y_ADDR,
        COMMIT_QUEUE_WORK_GLYPH_ADDR,
    ] {
        b.emit(Instr::LdAFromDirect {
            addr: direct(addr.get()),
        });
        b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
        b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    }
    for _ in 0..3 {
        b.emit(Instr::Ld8HlFromImm { imm: 0 });
        b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    }
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::C,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_TAIL_ADDR.get()),
    });
}

pub fn emit_commit_drain_hblank(b: &mut Builder) {
    emit_mode_guard_or_fault(b, HBLANK_PREFIX, PpuMode::HBlank);
    emit_queue_empty_check_ret(b);
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    emit_load_slot_hl_from_index_in_a(b);
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(UiCommitOpKind::PutGlyphCell as u8),
    });
    branch_jump(b, HBLANK_PREFIX, "put_glyph", Some(Cond::Z));
    b.emit(Instr::Ret { cond: None });

    b.label(symbol(HBLANK_PREFIX, "put_glyph"));
    emit_copy_slot_glyph_fields_to_work(b);
    emit_store_glyph_cell_from_work(b);
    publish_next_head_after_payload(b);
}

pub fn emit_commit_drain_vblank(b: &mut Builder) {
    emit_mode_guard_or_fault(b, VBLANK_PREFIX, PpuMode::VBlank);
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: UiCommitPlan::default_v1().max_ops_per_frame as u8,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR.get()),
    });
    b.label(symbol(VBLANK_PREFIX, "loop"));
    emit_mode_check_ret_if_not(b, VBLANK_PREFIX, "loop_mode_ok", PpuMode::VBlank);
    emit_queue_empty_check_ret(b);
    b.emit(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    emit_load_slot_hl_from_index_in_a(b);
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(UiCommitOpKind::PutOamSprite as u8),
    });
    branch_jump(b, VBLANK_PREFIX, "sprite", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(UiCommitOpKind::SetDmgPalette as u8),
    });
    branch_jump(b, VBLANK_PREFIX, "palette", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(UiCommitOpKind::PutGlyphCell as u8),
    });
    branch_jump(b, VBLANK_PREFIX, "put_glyph", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(UiCommitOpKind::FillGlyphRun as u8),
    });
    branch_jump(b, VBLANK_PREFIX, "fill_run", Some(Cond::Z));
    branch_jump(b, VBLANK_PREFIX, "publish", None);

    b.label(symbol(VBLANK_PREFIX, "put_glyph"));
    emit_copy_slot_glyph_fields_to_work(b);
    emit_store_glyph_cell_from_work(b);
    branch_jump(b, VBLANK_PREFIX, "publish", None);

    b.label(symbol(VBLANK_PREFIX, "fill_run"));
    emit_copy_slot_run_fields_to_work(b);
    emit_fill_run_from_work(b, VBLANK_PREFIX);
    branch_jump(b, VBLANK_PREFIX, "publish", None);

    b.label(symbol(VBLANK_PREFIX, "palette"));
    emit_copy_slot_palette_fields_to_work(b);
    emit_store_palette_from_work(b, VBLANK_PREFIX);
    branch_jump(b, VBLANK_PREFIX, "publish", None);

    b.label(symbol(VBLANK_PREFIX, "sprite"));
    emit_copy_slot_sprite_fields_to_work(b);
    emit_store_sprite_from_work(b, VBLANK_PREFIX);

    b.label(symbol(VBLANK_PREFIX, "publish"));
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR.get()),
    });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR.get()),
    });
    publish_next_head_after_payload(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_FRAME_REMAINING_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    branch_jump(b, VBLANK_PREFIX, "loop", Some(Cond::NZ));
}

pub fn emit_bootstrap_vram_init(b: &mut Builder) {
    b.emit(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new((gbf_hw::lcd::LCDC_REG & 0x00FF) as u8),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    b.emit(Instr::Ret {
        cond: Some(Cond::NZ),
    });
    emit_copy_font_to_vram(b);
    emit_clear_bg_map(b);
}

fn emit_compute_next_tail_or_fault(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_TAIL_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(COMMIT_QUEUE_LEN - 1),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_HEAD_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Reg(Reg8::C),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::NZ),
        off: 7,
    });
    emit_call_panic_fault(b, queue_full_fault());
    b.emit(Instr::Ret { cond: None });
}

fn emit_queue_empty_check_ret(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_HEAD_ADDR.get()),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_TAIL_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    b.emit(Instr::Ret {
        cond: Some(Cond::Z),
    });
}

fn emit_mode_guard_or_fault(b: &mut Builder, prefix: &'static str, expected: PpuMode) {
    emit_lcdc_enabled_bit(b);
    branch_jump(b, prefix, "mode_ok", Some(Cond::Z));
    emit_read_stat_mode(b);
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(expected as u8),
    });
    branch_jump(b, prefix, "mode_ok", Some(Cond::Z));
    emit_call_panic_fault(b, illegal_mode_fault());
    b.emit(Instr::Ret { cond: None });
    b.label(symbol(prefix, "mode_ok"));
}

fn emit_call_panic_fault(b: &mut Builder, fault: FaultCode) {
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: fault as u16,
    });
    b.branch(SymbolicBranch::call(
        SymbolName::runtime("panic", "entry").expect("static symbol"),
        None,
    ));
}

fn emit_mode_check_ret_if_not(
    b: &mut Builder,
    prefix: &'static str,
    ok_label: &'static str,
    expected: PpuMode,
) {
    emit_lcdc_enabled_bit(b);
    branch_jump(b, prefix, ok_label, Some(Cond::Z));
    emit_read_stat_mode(b);
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(expected as u8),
    });
    branch_jump(b, prefix, ok_label, Some(Cond::Z));
    b.emit(Instr::Ret { cond: None });
    b.label(symbol(prefix, ok_label));
}

fn emit_lcdc_enabled_bit(b: &mut Builder) {
    b.emit(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new((gbf_hw::lcd::LCDC_REG & 0x00FF) as u8),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
}

fn emit_load_slot_hl_from_index_in_a(b: &mut Builder) {
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::H,
        imm: (COMMIT_QUEUE_BASE_ADDR.get() >> 8) as u8,
    });
    for _ in 0..3 {
        b.emit(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    b.emit(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
}

fn emit_copy_slot_glyph_fields_to_work(b: &mut Builder) {
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_X_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_Y_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
}

fn emit_copy_slot_run_fields_to_work(b: &mut Builder) {
    emit_copy_slot_glyph_fields_to_work(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(MAX_FILL_GLYPH_RUN_CELLS + 1),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::C),
        off: 2,
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: MAX_FILL_GLYPH_RUN_CELLS,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_LEN_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
}

fn emit_copy_slot_palette_fields_to_work(b: &mut Builder) {
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_TARGET_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_VALUE_ADDR.get()),
    });
}

fn emit_copy_slot_sprite_fields_to_work(b: &mut Builder) {
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_SPRITE_INDEX_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_Y_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_X_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::Ld8RegFromHl { dst: Reg8::A });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_ATTRS_ADDR.get()),
    });
}

fn emit_compute_bg_addr_hl_from_work_xy(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_Y_ADDR.get()),
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::H,
        imm: (BOOTSTRAP_BG_MAP_ORIGIN >> 8) as u8,
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(16),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::C),
        off: 4,
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    b.emit(Instr::SubA {
        src: AluSrc8::Imm(16),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(8),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::C),
        off: 3,
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    b.emit(Instr::SubA {
        src: AluSrc8::Imm(8),
    });
    for _ in 0..5 {
        b.emit(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    b.emit(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_X_ADDR.get()),
    });
    b.emit(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    b.emit(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::NC),
        off: 1,
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
}

fn emit_store_glyph_cell_from_work(b: &mut Builder) {
    emit_compute_bg_addr_hl_from_work_xy(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
    b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
}

fn emit_fill_run_from_work(b: &mut Builder, prefix: &'static str) {
    emit_compute_bg_addr_hl_from_work_xy(b);
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_LEN_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    branch_jump(b, prefix, "publish", Some(Cond::Z));
    b.label(symbol(prefix, "fill_loop"));
    emit_mode_check_ret_if_not(b, prefix, "fill_mode_ok", PpuMode::VBlank);
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_GLYPH_ADDR.get()),
    });
    b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
    b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_LEN_ADDR.get()),
    });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_WORK_LEN_ADDR.get()),
    });
    branch_jump(b, prefix, "fill_loop", Some(Cond::NZ));
}

fn emit_store_palette_from_work(b: &mut Builder, prefix: &'static str) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_TARGET_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    branch_jump(b, prefix, "palette_bgp", Some(Cond::Z));
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(1),
    });
    branch_jump(b, prefix, "palette_obp0", Some(Cond::Z));
    branch_jump(b, prefix, "palette_obp1", None);

    b.label(symbol(prefix, "palette_bgp"));
    emit_store_palette_value(b, gbf_hw::lcd::BGP_REG);
    branch_jump(b, prefix, "publish", None);

    b.label(symbol(prefix, "palette_obp0"));
    emit_store_palette_value(b, gbf_hw::lcd::OBP0_REG);
    branch_jump(b, prefix, "publish", None);

    b.label(symbol(prefix, "palette_obp1"));
    emit_store_palette_value(b, gbf_hw::lcd::OBP1_REG);
}

fn emit_store_palette_value(b: &mut Builder, addr: u16) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_VALUE_ADDR.get()),
    });
    b.emit(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new((addr & 0x00FF) as u8),
    });
}

fn emit_store_sprite_from_work(b: &mut Builder, prefix: &'static str) {
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: gbf_hw::memory::OAM_BASE,
    });
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::DE,
        imm: 4,
    });
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_WORK_SPRITE_INDEX_ADDR.get()),
    });
    b.emit(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::Z),
        off: 4,
    });
    b.emit(Instr::AddHl { src: Reg16Data::DE });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::JrRel {
        cond: Some(Cond::NZ),
        off: -4,
    });
    for (addr, label) in [
        (COMMIT_QUEUE_WORK_Y_ADDR, "sprite_y_mode_ok"),
        (COMMIT_QUEUE_WORK_X_ADDR, "sprite_x_mode_ok"),
        (COMMIT_QUEUE_WORK_GLYPH_ADDR, "sprite_tile_mode_ok"),
        (COMMIT_QUEUE_WORK_ATTRS_ADDR, "sprite_attrs_mode_ok"),
    ] {
        emit_mode_check_ret_if_not(b, prefix, label, PpuMode::VBlank);
        b.emit(Instr::LdAFromDirect {
            addr: direct(addr.get()),
        });
        b.emit(Instr::Ld8HlFromReg { src: Reg8::A });
        b.emit(Instr::Inc16 { dst: Reg16Data::HL });
    }
}

fn emit_copy_font_to_vram(b: &mut Builder) {
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: crate::TEXT_FONT_DATA_ADDR,
    });
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::DE,
        imm: gbf_hw::memory::VRAM_BASE,
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::B,
        imm: 8,
    });
    b.label(symbol(BOOTSTRAP_PREFIX, "font_outer"));
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::C,
        imm: 0,
    });
    b.label(symbol(BOOTSTRAP_PREFIX, "font_inner"));
    b.emit(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    b.emit(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    b.emit(Instr::Inc16 { dst: Reg16Data::DE });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    branch_jump(b, BOOTSTRAP_PREFIX, "font_inner", Some(Cond::NZ));
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    branch_jump(b, BOOTSTRAP_PREFIX, "font_outer", Some(Cond::NZ));
}

fn emit_clear_bg_map(b: &mut Builder) {
    // Keep one fixed VRAM write visible to the static effect audit; the loop clears the full map.
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: 0,
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(BOOTSTRAP_BG_MAP_ORIGIN),
    });
    b.emit(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: BOOTSTRAP_BG_MAP_ORIGIN,
    });
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::B,
        imm: (BOOTSTRAP_BG_MAP_BYTES / 256) as u8,
    });
    b.label(symbol(BOOTSTRAP_PREFIX, "bg_clear_outer"));
    b.emit(Instr::Ld8RegFromImm {
        dst: Reg8::C,
        imm: 0,
    });
    b.label(symbol(BOOTSTRAP_PREFIX, "bg_clear_inner"));
    b.emit(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    branch_jump(b, BOOTSTRAP_PREFIX, "bg_clear_inner", Some(Cond::NZ));
    b.emit(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    branch_jump(b, BOOTSTRAP_PREFIX, "bg_clear_outer", Some(Cond::NZ));
}

fn emit_read_stat_mode(b: &mut Builder) {
    b.emit(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new((gbf_hw::lcd::STAT_REG & 0x00FF) as u8),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(0b11),
    });
}

fn publish_next_head_after_payload(b: &mut Builder) {
    b.emit(Instr::LdAFromDirect {
        addr: direct(COMMIT_QUEUE_HEAD_ADDR.get()),
    });
    b.emit(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    b.emit(Instr::AndA {
        src: AluSrc8::Imm(COMMIT_QUEUE_LEN - 1),
    });
    b.emit(Instr::LdDirectFromA {
        addr: direct(COMMIT_QUEUE_HEAD_ADDR.get()),
    });
}

fn branch_jump(b: &mut Builder, module: &'static str, target: &'static str, cond: Option<Cond>) {
    b.branch(SymbolicBranch::jump(symbol(module, target), cond));
}

fn symbol(module: &'static str, target: &'static str) -> SymbolName {
    SymbolName::runtime(module, target).expect("static symbol")
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("video commit direct address is below high memory")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::section_effect_kinds;
    use gbf_asm::effect::MachineEffectKind;

    #[test]
    fn ui_commit_op_exhaustive() {
        let kinds = [
            op_kind(UiCommitOp::PutGlyphCell {
                x: 0,
                y: 0,
                glyph: 0,
            }),
            op_kind(UiCommitOp::FillGlyphRun {
                x: 0,
                y: 0,
                len: 1,
                glyph: 0,
            }),
            op_kind(UiCommitOp::SetDmgPalette {
                target: DmgPaletteRegister::Bgp,
                value: 0,
            }),
            op_kind(UiCommitOp::PutOamSprite {
                sprite_index: 0,
                y: 0,
                x: 0,
                tile: 0,
                attrs: 0,
            }),
        ];
        assert_eq!(
            kinds,
            [
                UiCommitOpKind::PutGlyphCell,
                UiCommitOpKind::FillGlyphRun,
                UiCommitOpKind::SetDmgPalette,
                UiCommitOpKind::PutOamSprite,
            ]
        );
    }

    #[test]
    fn wire_op_size_is_8_bytes() {
        assert_eq!(UI_COMMIT_WIRE_OP_BYTES, 8);
        assert_eq!(core::mem::size_of::<UiCommitWireOp>(), 8);
    }

    #[test]
    fn fill_run_wire_len_is_bounded() {
        assert_eq!(bounded_fill_run_len(0), 0);
        assert_eq!(bounded_fill_run_len(MAX_FILL_GLYPH_RUN_CELLS), 20);
        assert_eq!(bounded_fill_run_len(255), MAX_FILL_GLYPH_RUN_CELLS);
        let wire = UiCommitWireOp::encode(UiCommitOp::FillGlyphRun {
            x: 0,
            y: 0,
            len: 255,
            glyph: b' ',
        })
        .bytes();
        assert_eq!(wire[4], MAX_FILL_GLYPH_RUN_CELLS);
    }

    #[test]
    fn hblank_drain_max_one_op() {
        assert_eq!(UiCommitPlan::default_v1().max_ops_per_hblank, 1);
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "hblank_cap").unwrap(),
        );
        emit_commit_drain_hblank(&mut builder);
        let immediates: Vec<u8> = builder
            .finish()
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::CpA {
                    src: AluSrc8::Imm(imm),
                } => Some(imm),
                _ => None,
            })
            .collect();
        assert!(immediates.contains(&(UiCommitOpKind::PutGlyphCell as u8)));
        assert!(!immediates.contains(&(UiCommitOpKind::FillGlyphRun as u8)));
        assert!(!immediates.contains(&(UiCommitOpKind::PutOamSprite as u8)));
    }

    #[test]
    fn no_oam_writes_in_hblank() {
        assert!(!op_legal_in_hblank_drain(
            UiCommitOpKind::PutOamSprite,
            PpuMode::HBlank
        ));
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "hblank").unwrap(),
        );
        emit_commit_drain_hblank(&mut builder);
        let section = builder.finish();
        assert!(
            !section_effect_kinds(&section).contains(&MachineEffectKind::StoreToOam),
            "HBlank drain must not write OAM in M0"
        );
    }

    #[test]
    fn no_writes_in_mode_3() {
        assert!(!op_legal_in_vblank_drain(
            UiCommitOpKind::PutGlyphCell,
            PpuMode::Drawing
        ));
        assert!(!op_legal_in_vblank_drain(
            UiCommitOpKind::PutOamSprite,
            PpuMode::Drawing
        ));
    }

    #[test]
    fn oam_only_in_modes_0_1() {
        assert!(op_legal_in_vblank_drain(
            UiCommitOpKind::PutOamSprite,
            PpuMode::HBlank
        ));
        assert!(op_legal_in_vblank_drain(
            UiCommitOpKind::PutOamSprite,
            PpuMode::VBlank
        ));
        assert!(!op_legal_in_vblank_drain(
            UiCommitOpKind::PutOamSprite,
            PpuMode::OamSearch
        ));
    }

    #[test]
    fn vblank_priority() {
        assert_eq!(
            UiCommitPlan::default_v1().vblank_priority_ops,
            vec![UiCommitOpKind::PutOamSprite, UiCommitOpKind::SetDmgPalette]
        );
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "vblank_priority").unwrap(),
        );
        emit_commit_drain_vblank(&mut builder);
        let instrs: Vec<_> = builder
            .finish()
            .instrs()
            .iter()
            .map(|item| item.data)
            .collect();
        let opcode_read = instrs
            .iter()
            .position(|instr| matches!(instr, Instr::Ld8RegFromHl { dst: Reg8::A }))
            .expect("vblank drain reads op kind from slot");
        let op_checks: Vec<u8> = instrs[opcode_read + 1..]
            .iter()
            .filter_map(|instr| match instr {
                Instr::CpA {
                    src: AluSrc8::Imm(imm),
                } if (1..=4).contains(imm) => Some(*imm),
                _ => None,
            })
            .take(4)
            .collect();
        assert_eq!(
            op_checks,
            vec![
                UiCommitOpKind::PutOamSprite as u8,
                UiCommitOpKind::SetDmgPalette as u8,
                UiCommitOpKind::PutGlyphCell as u8,
                UiCommitOpKind::FillGlyphRun as u8,
            ]
        );
    }

    #[test]
    fn queue_full_raises_typed_fault() {
        assert_eq!(queue_full_fault(), FaultCode::UiCommitQueueFull);
        assert_ne!(queue_full_fault(), FaultCode::UiCommitOutsideLegalMode);
    }

    #[test]
    fn illegal_mode_write_raises_typed_fault() {
        assert_eq!(illegal_mode_fault(), FaultCode::UiCommitOutsideLegalMode);
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "illegal_mode").unwrap(),
        );
        emit_commit_drain_vblank(&mut builder);
        let section = builder.finish();
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::Ld16Imm {
                    dst: Reg16Data::HL,
                    imm
                } if imm == FaultCode::UiCommitOutsideLegalMode as u16
            )
        }));
        assert!(
            section
                .branches()
                .iter()
                .any(|item| { item.data.target == SymbolName::runtime("panic", "entry").unwrap() })
        );
    }

    #[test]
    fn enqueue_publishes_tail_last() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "enqueue").unwrap(),
        );
        emit_queue_op(
            &mut builder,
            UiCommitOp::PutGlyphCell {
                x: 1,
                y: 2,
                glyph: 3,
            },
        );
        let stores: Vec<_> = builder
            .finish()
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::LdDirectFromA { addr } => Some(addr.get()),
                _ => None,
            })
            .collect();
        assert_eq!(stores.last().copied(), Some(COMMIT_QUEUE_TAIL_ADDR.get()));
    }

    #[test]
    fn dynamic_glyph_enqueue_publishes_tail_last() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "enqueue_dynamic").unwrap(),
        );
        emit_queue_glyph_cell_from_regs(&mut builder);
        let section = builder.finish();
        let stores: Vec<_> = section
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::LdDirectFromA { addr } => Some(addr.get()),
                _ => None,
            })
            .collect();
        assert!(stores.contains(&COMMIT_QUEUE_WORK_X_ADDR.get()));
        assert!(stores.contains(&COMMIT_QUEUE_WORK_Y_ADDR.get()));
        assert!(stores.contains(&COMMIT_QUEUE_WORK_GLYPH_ADDR.get()));
        assert_eq!(stores.last().copied(), Some(COMMIT_QUEUE_TAIL_ADDR.get()));
        assert!(
            section
                .instrs()
                .iter()
                .any(|item| { matches!(item.data, Instr::Ld8HlFromReg { src: Reg8::A }) })
        );
    }

    #[test]
    fn drain_publishes_head_after_payload() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "drain").unwrap(),
        );
        emit_commit_drain_vblank(&mut builder);
        let stores: Vec<_> = builder
            .finish()
            .instrs()
            .iter()
            .filter_map(|item| match item.data {
                Instr::LdDirectFromA { addr } => Some(addr.get()),
                _ => None,
            })
            .collect();
        assert_eq!(stores.last().copied(), Some(COMMIT_QUEUE_HEAD_ADDR.get()));
    }

    #[test]
    fn fill_run_runtime_len_is_clamped_before_loop() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "fill_clamp").unwrap(),
        );
        emit_copy_slot_run_fields_to_work(&mut builder);
        let section = builder.finish();
        assert!(section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm
                } if imm == MAX_FILL_GLYPH_RUN_CELLS
            )
        }));
    }

    #[test]
    fn bootstrap_runs_with_lcd_off() {
        let mut builder = Builder::new(
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("test", "bootstrap").unwrap(),
        );
        emit_bootstrap_vram_init(&mut builder);
        let section = builder.finish();
        assert!(matches!(
            section.instrs()[0].data,
            Instr::LdAFromHighDirect { .. }
        ));
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
        assert!(!section.instrs().iter().any(|item| {
            matches!(
                item.data,
                Instr::LdDirectFromA { addr } if addr.get() == gbf_hw::memory::VRAM_BASE
            )
        }));
    }

    #[test]
    fn sole_vram_writer() {
        let section = build_video_commit_section();
        assert_eq!(
            section.privilege().execution_context,
            ExecutionContext::VideoCommitOnly
        );
        assert!(section_effect_kinds(&section).contains(&MachineEffectKind::StoreToVram));
    }

    #[test]
    fn sole_oam_writer() {
        assert!(
            section_effect_kinds(&build_video_commit_section())
                .contains(&MachineEffectKind::StoreToDynamic)
        );
    }
}
