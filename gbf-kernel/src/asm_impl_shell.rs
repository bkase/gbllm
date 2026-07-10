//! Interactive generation shell ROM (bd-1kbv1, v0 demo scope): boot + text
//! rendering, an on-screen charset-80 keyboard driven by the polled joypad,
//! prompt warmup through the stateful LinearState forward pass, and sampled
//! generation rendered to a BG-map transcript at token boundaries.
//!
//! This is the **v0 demo shell**, not the full M5 cooperative scheduler:
//!
//! - **Interrupts stay disabled for the whole session** (`di` at entry, no
//!   `ei` ever). The V3 weight chunks repurpose SP as a pop-stream pointer,
//!   so an ISR firing mid-kernel would corrupt the weight stream. The
//!   stated v0 policy "DI during token compute, EI only at token
//!   boundaries" is realized conservatively: UI work happens *only* at
//!   token boundaries (and in the idle input loop), and VBlank is found by
//!   polling LY instead of via the VBlank interrupt, so IME can stay off
//!   throughout. DeadlineAware yield integration inside kernels is the
//!   planv0 scheduler beads' follow-up.
//! - **VRAM discipline**: all tile/BG-map writes happen either with the LCD
//!   off (boot init) or inside the VBlank window found by polling LY
//!   (`ui_wait_vbl` busy-waits for the LY=144 edge; every UI routine writes
//!   well under the ~1140 M-cycle VBlank budget).
//! - **UI cadence**: between token boundaries the screen is static for one
//!   full forward pass (~3.5 s of DMG time at the S5 checkpoint). The
//!   per-token transcript glyph plus a block cursor at the next cell is the
//!   progress affordance; during prompt warmup the consumed prompt chars
//!   are highlighted (inverted) one by one at the same cadence.
//!
//! # Screen layout (20x18 BG cells, map at 0x9800)
//!
//! - rows 0..=9: transcript (200 cells); generated glyphs appear here, a
//!   newline id advances to the next row, and an inverted-space block
//!   cursor marks the next cell.
//! - row 11: prompt echo (up to [`SHELL_PROMPT_CAP`] chars).
//! - row 12: "GENERATING" message while a generation runs.
//! - rows 13..=16: the keyboard — a single 4x19 grid holding all 76
//!   charset_v1 ids in id order (cell index == charset id); the cursor is
//!   the inverted glyph.
//! - row 17: static help line "A:KEY B:DEL ST:GO".
//!
//! # Controls (polled once per frame while idle)
//!
//! D-pad moves the keyboard cursor (clamped to the grid), A types the
//! selected char, B is backspace, START submits the prompt. **planv0
//! InteractionBundle divergences (documented v0 simplifications):** a
//! single-page grid instead of the three-page lowercase/uppercase/symbols
//! keyboard, B is backspace instead of one-shot shift (all 76 ids are
//! directly reachable on the single page, so no shift is needed), and
//! SELECT/page cycling does not exist.
//!
//! # Tile mapping
//!
//! Tile index == charset id for the 76 glyphs (0..=75); tiles 128..=203 are
//! the same glyphs inverted (both bitplanes complemented), so highlighting
//! is one BG byte write. Tile 62 (space) is blank; tile 190 (inverted
//! space) is the solid block cursor. The 8x8 font is supplied by the caller
//! ([`SHELL_FONT_BYTES`] bytes, 16 per glyph in charset id order) — the
//! bench harness reuses the committed M0 runtime font asset.
//!
//! # Generation semantics (must mirror the host evaluator byte-exactly)
//!
//! On START (with a nonempty prompt): the recurrent state is zeroed (fresh
//! context per submit), the XorShift16 state at
//! [`crate::asm_impl_state::S_RNG_ADDR`] is canonicalized (0 -> 1) but
//! otherwise left as poked/carried, every prompt id is fed through one
//! forward pass each (state warmup, **no RNG draws**), and then tokens are
//! sampled from the current logits with the pinned integer top-k sampler,
//! rendered, written to the output ring at
//! [`crate::asm_impl_state::S_OUT_BASE`], and fed back — until
//! `n_gen_tokens` tokens or the transcript region is full.

use gbf_asm::isa::{AluSrc8, BitIndex, CbTarget, Cond, HighDirectOffset, Instr, Reg8, Reg16Data};
use gbf_asm::rom::{ENTRY_POINT, RomSize};

use crate::asm_impl_model::{
    BANK_BYTES, CHUNK_ENTRY, MBC5_ROMB0, MBC5_ROMB1, ModelAsm, ModelRomError, a_from, a_to,
    ld_r_imm, ld_rr, ld16,
};
use crate::asm_impl_state::{
    S_INPUT_ADDR, S_RNG_ADDR, S_SAMPLED_ADDR, S_STACK_TOP, ShellWram, StateWramLayout,
    WeightLowering, assemble_state_rom, emit_state_forward_body, emit_state_routines_and_tables,
    emit_zero16, plan_state_rom_with, set_bank,
};
use crate::state_model_ref::IntStateLoweredModel;

// ---------------------------------------------------------------------------
// WRAM map (shell-owned block; disjoint from every stateful-ROM buffer)
// ---------------------------------------------------------------------------

/// Maximum prompt length (one BG row).
pub const SHELL_PROMPT_CAP: u8 = 20;

// ---------------------------------------------------------------------------
// screen geometry and tile mapping
// ---------------------------------------------------------------------------

pub const BG_MAP_BASE: u16 = 0x9800;
pub const BG_MAP_STRIDE: u16 = 32;
/// Transcript region: rows 0..TRANSCRIPT_ROWS, cols 0..TRANSCRIPT_COLS.
pub const TRANSCRIPT_ROWS: u8 = 10;
pub const TRANSCRIPT_COLS: u8 = 20;
pub const TRANSCRIPT_CELLS: u8 = 200;
pub const PROMPT_ROW: u8 = 11;
pub const MSG_ROW: u8 = 12;
/// Keyboard grid: 4 rows x 19 cols starting at this BG row.
pub const KB_ORIGIN_ROW: u8 = 13;
pub const KB_ROWS: u8 = 4;
pub const KB_COLS: u8 = 19;
pub const KB_CELLS: u8 = 76;
pub const STATUS_ROW: u8 = 17;
/// Inverted-glyph tile index offset.
pub const SHELL_INVERT_TILE_OFFSET: u8 = 128;
/// charset_v1 space id (blank tile).
pub const SHELL_SPACE_ID: u8 = 62;
/// charset_v1 newline id (advances the transcript row).
pub const SHELL_NEWLINE_ID: u8 = 75;
/// Block cursor tile: inverted space.
pub const SHELL_CURSOR_TILE: u8 = SHELL_INVERT_TILE_OFFSET + SHELL_SPACE_ID;
/// Font payload: 76 glyphs x 16 bytes (2bpp 8x8), charset id order.
pub const SHELL_FONT_TILES: usize = 76;
pub const SHELL_FONT_BYTES: usize = SHELL_FONT_TILES * 16;
/// Maximum generation length (the transcript region size; also <= the
/// 256-byte output ring).
pub const SHELL_MAX_GEN_TOKENS: u8 = TRANSCRIPT_CELLS;

/// Status row text "A:KEY B:DEL ST:GO" as charset_v1 ids.
pub const SHELL_STATUS_TEXT_IDS: [u8; 17] =
    [0, 69, 10, 4, 24, 62, 1, 69, 3, 4, 11, 62, 18, 19, 69, 6, 14];
/// Message row text "GENERATING" as charset_v1 ids.
pub const SHELL_MSG_TEXT_IDS: [u8; 10] = [6, 4, 13, 4, 17, 0, 19, 8, 13, 6];

// I/O registers (LDH offsets).
const IO_JOYP: u8 = 0x00;
const IO_LCDC: u8 = 0x40;
const IO_SCY: u8 = 0x42;
const IO_SCX: u8 = 0x43;
const IO_LY: u8 = 0x44;
const IO_BGP: u8 = 0x47;

/// LCDC: LCD on | BG tile data 0x8000 | BG map 0x9800 | BG on.
const LCDC_ON: u8 = 0x91;
/// Standard DMG palette (3=darkest .. 0=lightest).
const BGP_STANDARD: u8 = 0xE4;

// ---------------------------------------------------------------------------
// Inference "thinking" animation (Aurora Plasma Drift)
// ---------------------------------------------------------------------------
//
// During a token's ~18s compute the screen would otherwise be frozen. The
// weight-chunk loop calls `anim_tick` once per chunk (~22 Hz); it writes only
// PPU registers (never VRAM, no VBlank wait) so it is safe mid-compute. We fill
// the keyboard rows (idle during generation) with a seamless plasma field built
// from pixel values 1 and 2 ONLY, then shimmer it by rotating those two shades
// in BGP. Every LUT entry keeps pixel value 0 -> shade 0 (white paper) and
// value 3 -> shade 3 (black ink), so the transcript text stays black-on-white
// and perfectly legible the whole time.

/// First plasma tile index (52 free tiles 76..=127 available; we use 16).
const ANIM_TILE0: u8 = 76;
const ANIM_TILES: u8 = 16;
/// Plasma panel rows in the BG map (the keyboard grid, unused during compute).
const ANIM_ROW_LO: u8 = KB_ORIGIN_ROW; // 13
const ANIM_ROW_HI: u8 = KB_ORIGIN_ROW + KB_ROWS; // 17 (exclusive)

/// 8-step BGP shimmer. Text-safe: bits 0-1 (value 0) stay 00 and bits 6-7
/// (value 3) stay 11 in every entry; only values 1 and 2 rotate.
const ANIM_BGP_LUT: [u8; 8] = [0xE4, 0xD8, 0xD4, 0xF8, 0xEC, 0xE8, 0xC4, 0xD0];

/// 16 plasma tiles: a seamless 32x32 (4x4-tile) interference cell, 2bpp planar,
/// using only pixel values 1 and 2. Tile (tr,tc) -> index ANIM_TILE0+tr*4+tc.
fn anim_plasma_tiles() -> Vec<u8> {
    use core::f64::consts::TAU;
    let field = |x: i32, y: i32| -> f64 {
        let hyp = (((x - 16).pow(2) + (y - 16).pow(2)) as f64).sqrt();
        (TAU * x as f64 / 32.0).sin()
            + (TAU * y as f64 / 32.0).sin()
            + (TAU * (x + y) as f64 / 32.0).sin()
            + (TAU * hyp / 32.0).sin()
    };
    let mut out = Vec::with_capacity(ANIM_TILES as usize * 16);
    for tr in 0..4i32 {
        for tc in 0..4i32 {
            for r in 0..8i32 {
                let (mut lo, mut hi) = (0u8, 0u8);
                for c in 0..8i32 {
                    let pix = if field(tc * 8 + c, tr * 8 + r) >= 0.0 {
                        1
                    } else {
                        2
                    };
                    let bit = 7 - c;
                    if pix == 1 {
                        lo |= 1 << bit;
                    } else {
                        hi |= 1 << bit;
                    }
                }
                out.push(lo);
                out.push(hi);
            }
        }
    }
    out
}

/// BG-map bytes for the plasma panel: rows ANIM_ROW_LO..ANIM_ROW_HI x 20 cols,
/// each cell = ANIM_TILE0 + (row&3)*4 + (col&3) so the field tiles seamlessly.
fn anim_plasma_map() -> Vec<u8> {
    let mut m = Vec::new();
    for row in ANIM_ROW_LO..ANIM_ROW_HI {
        for col in 0..TRANSCRIPT_COLS {
            m.push(ANIM_TILE0 + (row & 3) * 4 + (col & 3));
        }
    }
    m
}

/// A fully assembled interactive shell ROM plus the facts and trap PCs the
/// runner needs.
#[derive(Debug, Clone)]
pub struct ShellRom {
    pub rom: Vec<u8>,
    pub layout: StateWramLayout,
    /// Idle input loop head; hit exactly once per polled frame.
    pub idle_pc: u16,
    /// Hit once per prompt char after its warmup forward pass.
    pub warm_boundary_pc: u16,
    /// Hit once per generated token after its transcript render.
    pub token_boundary_pc: u16,
    /// Hit once when a generation run completes (before returning to idle).
    pub gen_done_pc: u16,
    pub n_gen_tokens: u8,
    pub rom_size: RomSize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub ui_bank_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
}

fn ldh_a_from(asm: &mut ModelAsm, offset: u8) {
    asm.i(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(offset),
    });
}

fn ldh_a_to(asm: &mut ModelAsm, offset: u8) {
    asm.i(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(offset),
    });
}

fn bit_reg(asm: &mut ModelAsm, bit: u8, reg: Reg8) {
    asm.i(Instr::Bit {
        bit: BitIndex::new(bit).expect("bit index 0..=7"),
        target: CbTarget::Reg(reg),
    });
}

fn set_reg(asm: &mut ModelAsm, bit: u8, reg: Reg8) {
    asm.i(Instr::Set {
        bit: BitIndex::new(bit).expect("bit index 0..=7"),
        target: CbTarget::Reg(reg),
    });
}

// ---------------------------------------------------------------------------
// UI bank routines (execute with the UI bank mapped at 0x4000)
// ---------------------------------------------------------------------------

/// `ui_wait_vbl`: busy-wait for the next VBlank *edge* by polling LY: first
/// leave any current VBlank (LY >= 144), then wait for LY == 144. One call
/// per frame gives the idle loop its frame cadence; no interrupts are used.
fn emit_ui_wait_vbl(asm: &mut ModelAsm) {
    asm.label("ui_wait_vbl");
    asm.label("uwv_out");
    ldh_a_from(asm, IO_LY);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(144),
    });
    asm.jr(Some(Cond::NC), "uwv_out");
    asm.label("uwv_in");
    ldh_a_from(asm, IO_LY);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(144),
    });
    asm.jr(Some(Cond::C), "uwv_in");
    asm.i(Instr::Ret { cond: None });
}

/// `ui_joypad`: once-per-frame JOYP read with active-low decode into the
/// active-high WRAM cache (flat port of the M0 runtime joypad reader:
/// directions into bits 4..=7, buttons into bits 0..=3, double-read
/// debounce, both select lines released afterwards).
fn emit_ui_joypad(asm: &mut ModelAsm, sh: &ShellWram) {
    asm.label("ui_joypad");
    a_from(asm, sh.joy_cur);
    a_to(asm, sh.joy_prev);
    // directions column
    ld_r_imm(asm, Reg8::A, 0x20);
    ldh_a_to(asm, IO_JOYP);
    ldh_a_from(asm, IO_JOYP);
    ldh_a_from(asm, IO_JOYP);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x0F),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, 0);
    for (src, dst) in [(2u8, 4u8), (3, 5), (1, 6), (0, 7)] {
        let skip = asm.fresh("ujp_skip");
        bit_reg(asm, src, Reg8::C);
        asm.jr(Some(Cond::Z), &skip);
        set_reg(asm, dst, Reg8::B);
        asm.label(&skip);
    }
    // buttons column
    ld_r_imm(asm, Reg8::A, 0x10);
    ldh_a_to(asm, IO_JOYP);
    ldh_a_from(asm, IO_JOYP);
    ldh_a_from(asm, IO_JOYP);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x0F),
    });
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.i(Instr::Cpl);
    a_to(asm, sh.joy_cur);
    ld_r_imm(asm, Reg8::A, 0x30);
    ldh_a_to(asm, IO_JOYP);
    asm.i(Instr::Ret { cond: None });
}

/// `ui_kb_addr`: A = keyboard cell index (0..=75) -> HL = BG map address of
/// that key. Clobbers A, B; preserves C, D, E.
fn emit_ui_kb_addr(asm: &mut ModelAsm) {
    let base = BG_MAP_BASE + u16::from(KB_ORIGIN_ROW) * BG_MAP_STRIDE; // 0x99A0
    asm.label("ui_kb_addr");
    ld_r_imm(asm, Reg8::B, 0);
    asm.label("uka_div");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_COLS),
    });
    asm.jr(Some(Cond::C), "uka_done");
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(KB_COLS),
    });
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, "uka_div");
    asm.label("uka_done");
    // A = col, B = row (0..=3); offs = row*32 + col <= 114
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::B);
    for _ in 0..5 {
        asm.i(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((base & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (base >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Ret { cond: None });
}

/// `ui_cell_addr`: A = transcript cell (0..=199) -> HL = BG map address.
/// Clobbers A, B; preserves C, D, E.
fn emit_ui_cell_addr(asm: &mut ModelAsm) {
    asm.label("ui_cell_addr");
    ld_r_imm(asm, Reg8::B, 0);
    asm.label("uca_div");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.jr(Some(Cond::C), "uca_done");
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, "uca_div");
    asm.label("uca_done");
    // A = col, B = row (0..=9)
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::B);
    for _ in 0..4 {
        asm.i(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    } // row*16 <= 144, no carry
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // row*32 low byte, CF = bit 8
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_r_imm(asm, Reg8::A, (BG_MAP_BASE >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    }); // + col (row*32 low <= 0xE0 for rows with nonzero low byte; no carry)
    ld_rr(asm, Reg8::L, Reg8::A);
    asm.i(Instr::Ret { cond: None });
}

/// `ui_kb_move`: E = old cell index, C = new cell index. Re-draws the old
/// key normal and the new key inverted, and stores the new cursor.
fn emit_ui_kb_move(asm: &mut ModelAsm, sh: &ShellWram) {
    asm.label("ui_kb_move");
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, sh.kbcur);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.call("ui_kb_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.call("ui_kb_addr");
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(SHELL_INVERT_TILE_OFFSET),
    });
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_frame`: one idle-loop frame — VBlank edge, joypad poll, then the
/// keyboard step (cursor moves, type, backspace, submit). All BG writes
/// happen right after the VBlank edge, well inside the VBlank window.
fn emit_ui_frame(asm: &mut ModelAsm, sh: &ShellWram) {
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE; // 0x9960
    asm.label("ui_frame");
    asm.call("ui_wait_vbl");
    asm.call("ui_joypad");
    // D = newly pressed = CUR & !PREV
    a_from(asm, sh.joy_prev);
    asm.i(Instr::Cpl);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, sh.joy_cur);
    asm.i(Instr::AndA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::D, Reg8::A);

    // Right (bit 7): idx < 75 -> idx + 1
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::Z), "uf_no_r");
    a_from(asm, sh.kbcur);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_CELLS - 1),
    });
    asm.jr(Some(Cond::Z), "uf_no_r");
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::C),
    });
    asm.call("ui_kb_move");
    asm.label("uf_no_r");

    // Left (bit 6): idx > 0 -> idx - 1
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x40),
    });
    asm.jr(Some(Cond::Z), "uf_no_l");
    a_from(asm, sh.kbcur);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "uf_no_l");
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::C),
    });
    asm.call("ui_kb_move");
    asm.label("uf_no_l");

    // Up (bit 4): idx >= 19 -> idx - 19
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x10),
    });
    asm.jr(Some(Cond::Z), "uf_no_u");
    a_from(asm, sh.kbcur);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_COLS),
    });
    asm.jr(Some(Cond::C), "uf_no_u");
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(KB_COLS),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.call("ui_kb_move");
    asm.label("uf_no_u");

    // Down (bit 5): idx + 19 <= 75 -> idx + 19
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x20),
    });
    asm.jr(Some(Cond::Z), "uf_no_d");
    a_from(asm, sh.kbcur);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_CELLS - KB_COLS),
    });
    asm.jr(Some(Cond::NC), "uf_no_d");
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(KB_COLS),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.call("ui_kb_move");
    asm.label("uf_no_d");

    // A (bit 0): type the selected char
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x01),
    });
    asm.jr(Some(Cond::Z), "uf_no_a");
    a_from(asm, sh.plen);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(SHELL_PROMPT_CAP),
    });
    asm.jr(Some(Cond::NC), "uf_no_a");
    ld_rr(asm, Reg8::C, Reg8::A); // C = plen
    a_from(asm, sh.kbcur);
    ld_rr(asm, Reg8::E, Reg8::A); // E = id
    // prompt[plen] = id  (sh.prompt low byte is 0x00)
    ld_r_imm(asm, Reg8::H, (sh.prompt >> 8) as u8);
    ld_rr(asm, Reg8::L, Reg8::C);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    // echo to the prompt row
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((prompt_bg & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, (prompt_bg >> 8) as u8);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.plen);
    asm.label("uf_no_a");

    // B (bit 1): backspace
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x02),
    });
    asm.jr(Some(Cond::Z), "uf_no_b");
    a_from(asm, sh.plen);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "uf_no_b");
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.plen);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((prompt_bg & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, (prompt_bg >> 8) as u8);
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_SPACE_ID,
    });
    asm.label("uf_no_b");

    // START (bit 3): submit
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x08),
    });
    asm.jr(Some(Cond::Z), "uf_no_s");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, sh.submit);
    asm.label("uf_no_s");
    asm.i(Instr::Ret { cond: None });
}

/// `ui_warm_mark`: highlight (invert) prompt char [`sh.widx`] on the
/// prompt row — the warmup progress affordance.
fn emit_ui_warm_mark(asm: &mut ModelAsm, sh: &ShellWram) {
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE;
    asm.label("ui_warm_mark");
    asm.call("ui_wait_vbl");
    a_from(asm, sh.widx);
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::H, (sh.prompt >> 8) as u8);
    ld_rr(asm, Reg8::L, Reg8::C);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(SHELL_INVERT_TILE_OFFSET),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((prompt_bg & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, (prompt_bg >> 8) as u8);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_render_token`: render the sampled id at the transcript cursor. A
/// newline id erases the block cursor and advances to the next row start;
/// any other id writes its glyph and advances one cell. If the region is
/// now full, [`sh.tfull`] is set; otherwise the block cursor is drawn
/// at the new cell.
fn emit_ui_render_token(asm: &mut ModelAsm, sh: &ShellWram) {
    asm.label("ui_render_token");
    asm.call("ui_wait_vbl");
    a_from(asm, S_SAMPLED_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(SHELL_NEWLINE_ID),
    });
    asm.jr(Some(Cond::Z), "urt_nl");
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    a_from(asm, sh.tcur);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.tcur);
    asm.jr(None, "urt_after");
    asm.label("urt_nl");
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_SPACE_ID,
    });
    // new cell = (row + 1) * 20
    a_from(asm, sh.tcur);
    ld_r_imm(asm, Reg8::B, 0);
    asm.label("urt_div");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.jr(Some(Cond::C), "urt_dd");
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, "urt_div");
    asm.label("urt_dd");
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    }); // *5
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // *20
    a_to(asm, sh.tcur);
    asm.label("urt_after");
    a_from(asm, sh.tcur);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_CELLS),
    });
    asm.jr(Some(Cond::C), "urt_nf");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, sh.tfull);
    asm.i(Instr::Ret { cond: None });
    asm.label("urt_nf");
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_CURSOR_TILE,
    });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_gen_begin`: show the "GENERATING" message, clear the transcript
/// region (one row per VBlank), reset the transcript cursor, and draw the
/// block cursor at cell 0.
fn emit_ui_gen_begin(asm: &mut ModelAsm, sh: &ShellWram) {
    let msg_bg = BG_MAP_BASE + u16::from(MSG_ROW) * BG_MAP_STRIDE;
    asm.label("ui_gen_begin");
    asm.call("ui_wait_vbl");
    asm.ld16_label(Reg16Data::HL, "shell_msg_txt", 0);
    ld16(asm, Reg16Data::DE, msg_bg);
    ld_r_imm(asm, Reg8::B, SHELL_MSG_TEXT_IDS.len() as u8);
    asm.label("ugb_msg");
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::DE,
    });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "ugb_msg");
    // clear the transcript, one row per VBlank
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.ui_row);
    asm.label("ugb_row");
    asm.call("ui_wait_vbl");
    a_from(asm, sh.ui_row);
    for _ in 0..4 {
        asm.i(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // row*32 low, CF = bit 8
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (BG_MAP_BASE >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_r_imm(asm, Reg8::B, TRANSCRIPT_COLS);
    ld_r_imm(asm, Reg8::A, SHELL_SPACE_ID);
    asm.label("ugb_fill");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "ugb_fill");
    a_from(asm, sh.ui_row);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.ui_row);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_ROWS),
    });
    asm.jp(Some(Cond::NZ), "ugb_row");
    // reset transcript state, draw the block cursor at cell 0
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.tcur);
    a_to(asm, sh.tfull);
    asm.call("ui_wait_vbl");
    ld16(asm, Reg16Data::HL, BG_MAP_BASE);
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_CURSOR_TILE,
    });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_gen_end`: clear the "GENERATING" message and the prompt row, and
/// reset the prompt length for the next entry.
fn emit_ui_gen_end(asm: &mut ModelAsm, sh: &ShellWram) {
    let msg_bg = BG_MAP_BASE + u16::from(MSG_ROW) * BG_MAP_STRIDE;
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE;
    asm.label("ui_gen_end");
    asm.call("ui_wait_vbl");
    ld16(asm, Reg16Data::HL, msg_bg);
    ld_r_imm(asm, Reg8::B, SHELL_MSG_TEXT_IDS.len() as u8);
    ld_r_imm(asm, Reg8::A, SHELL_SPACE_ID);
    asm.label("uge_msg");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "uge_msg");
    ld16(asm, Reg16Data::HL, prompt_bg);
    ld_r_imm(asm, Reg8::B, SHELL_PROMPT_CAP);
    asm.label("uge_prompt");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "uge_prompt");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.plen);
    asm.i(Instr::Ret { cond: None });
}

/// `ui_init`: LCD off (from inside VBlank), palette/scroll setup, font tile
/// upload (normal to 0x8000, inverted to 0x8800), full BG-map clear,
/// keyboard grid + status row + initial cursor, then LCD on.
fn emit_ui_init(asm: &mut ModelAsm, sh: &ShellWram) {
    let status_bg = BG_MAP_BASE + u16::from(STATUS_ROW) * BG_MAP_STRIDE;
    asm.label("ui_init");
    asm.call("ui_wait_vbl");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC);
    ldh_a_to(asm, IO_SCY);
    ldh_a_to(asm, IO_SCX);
    ld_r_imm(asm, Reg8::A, BGP_STANDARD);
    ldh_a_to(asm, IO_BGP);
    // font: normal tiles 0..=75 at 0x8000
    asm.ld16_label(Reg16Data::HL, "shell_font", 0);
    ld16(asm, Reg16Data::DE, 0x8000);
    ld16(asm, Reg16Data::BC, SHELL_FONT_BYTES as u16);
    asm.label("ui_fc1");
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::DE,
    });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "ui_fc1");
    // font: inverted tiles 128..=203 at 0x8800 (complement both planes)
    asm.ld16_label(Reg16Data::HL, "shell_font", 0);
    ld16(asm, Reg16Data::DE, 0x8800);
    ld16(asm, Reg16Data::BC, SHELL_FONT_BYTES as u16);
    asm.label("ui_fc2");
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Cpl);
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::DE,
    });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "ui_fc2");
    // clear the whole 32x32 BG map to spaces
    ld16(asm, Reg16Data::HL, BG_MAP_BASE);
    ld16(asm, Reg16Data::BC, 1024);
    asm.label("ui_cl");
    ld_r_imm(asm, Reg8::A, SHELL_SPACE_ID);
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "ui_cl");
    // keyboard grid: cell index == charset id == tile index
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("ui_kb");
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.call("ui_kb_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::C });
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_CELLS),
    });
    asm.jr(Some(Cond::NZ), "ui_kb");
    // status row
    asm.ld16_label(Reg16Data::HL, "shell_status_txt", 0);
    ld16(asm, Reg16Data::DE, status_bg);
    ld_r_imm(asm, Reg8::B, SHELL_STATUS_TEXT_IDS.len() as u8);
    asm.label("ui_st");
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::DE,
    });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "ui_st");
    // initial keyboard cursor: cell 0 inverted
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.kbcur);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.call("ui_kb_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_INVERT_TILE_OFFSET,
    });
    // LCD on
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC);
    asm.i(Instr::Ret { cond: None });
}

/// `anim_setup` / `anim_restore` (UI bank) + plasma tile/map data. Setup paints
/// the plasma field over the keyboard rows (LCD off for the bulk VRAM write);
/// restore repaints the keyboard and resets palette/scroll when generation ends.
/// The transcript rows are never touched, so the generated text is untouched.
fn emit_anim_ui(asm: &mut ModelAsm, sh: &ShellWram) {
    use gbf_asm::isa::{IncDec8Target, Reg16Addr};
    asm.label("anim_setup");
    asm.call("ui_wait_vbl");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC); // LCD off for the bulk VRAM write
    // plasma tiles -> VRAM at tile ANIM_TILE0 (0x8000 + 76*16 = 0x84C0)
    asm.ld16_label(Reg16Data::HL, "anim_tiles", 0);
    ld16(asm, Reg16Data::DE, 0x8000 + (ANIM_TILE0 as u16) * 16);
    ld16(asm, Reg16Data::BC, (ANIM_TILES as u16) * 16);
    asm.label("as_ct");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "as_ct");
    // plasma map: ANIM_ROW_LO..ANIM_ROW_HI, 20 cols, +12 gap to next map row
    asm.ld16_label(Reg16Data::HL, "anim_map", 0);
    ld16(
        asm,
        Reg16Data::DE,
        BG_MAP_BASE + (ANIM_ROW_LO as u16) * BG_MAP_STRIDE,
    );
    ld_r_imm(asm, Reg8::B, ANIM_ROW_HI - ANIM_ROW_LO);
    asm.label("as_row");
    ld_r_imm(asm, Reg8::C, TRANSCRIPT_COLS);
    asm.label("as_col");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "as_col");
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(BG_MAP_STRIDE as u8 - TRANSCRIPT_COLS),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "as_row");
    ld_r_imm(asm, Reg8::A, BGP_STANDARD);
    ldh_a_to(asm, IO_BGP);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_SCX);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_SCY);
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC);
    asm.i(Instr::Ret { cond: None });

    asm.label("anim_restore");
    asm.call("ui_wait_vbl");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC); // LCD off
    // repaint keyboard cells 0..KB_CELLS (cell index == charset id == tile id)
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("ar_kb");
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.call("ui_kb_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::C });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(KB_CELLS),
    });
    asm.jr(Some(Cond::NZ), "ar_kb");
    // redraw the keyboard cursor (inverted tile) at the current cell
    a_from(asm, sh.kbcur);
    asm.call("ui_kb_addr"); // HL = cell BG address (takes A = cell)
    a_from(asm, sh.kbcur);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(SHELL_INVERT_TILE_OFFSET),
    });
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    ld_r_imm(asm, Reg8::A, BGP_STANDARD);
    ldh_a_to(asm, IO_BGP);
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC);
    asm.i(Instr::Ret { cond: None });

    asm.label("anim_tiles");
    asm.bytes(anim_plasma_tiles());
    asm.label("anim_map");
    asm.bytes(anim_plasma_map());
}

/// Build the UI bank image (routines + font + text data) and return its
/// bytes plus the entry addresses bank-0 code calls.
fn build_ui_bank(font_tiles: &[u8], sh: &ShellWram) -> Result<(Vec<u8>, UiEntries), ModelRomError> {
    debug_assert_eq!(font_tiles.len(), SHELL_FONT_BYTES);
    let mut asm = ModelAsm::new(CHUNK_ENTRY);
    emit_ui_init(&mut asm, sh);
    emit_ui_frame(&mut asm, sh);
    emit_ui_warm_mark(&mut asm, sh);
    emit_ui_render_token(&mut asm, sh);
    emit_ui_gen_begin(&mut asm, sh);
    emit_ui_gen_end(&mut asm, sh);
    emit_ui_wait_vbl(&mut asm);
    emit_ui_joypad(&mut asm, sh);
    emit_ui_kb_addr(&mut asm);
    emit_ui_cell_addr(&mut asm);
    emit_ui_kb_move(&mut asm, sh);
    emit_anim_ui(&mut asm, sh);
    asm.label("shell_font");
    asm.bytes(font_tiles.to_vec());
    asm.label("shell_status_txt");
    asm.bytes(SHELL_STATUS_TEXT_IDS.to_vec());
    asm.label("shell_msg_txt");
    asm.bytes(SHELL_MSG_TEXT_IDS.to_vec());
    let (bytes, labels) = asm.finish()?;
    if bytes.len() > BANK_BYTES {
        return Err(ModelRomError::UiBankOverflow { bytes: bytes.len() });
    }
    let entries = UiEntries {
        init: labels["ui_init"],
        frame: labels["ui_frame"],
        warm_mark: labels["ui_warm_mark"],
        render_token: labels["ui_render_token"],
        gen_begin: labels["ui_gen_begin"],
        gen_end: labels["ui_gen_end"],
        anim_setup: labels["anim_setup"],
        anim_restore: labels["anim_restore"],
    };
    Ok((bytes, entries))
}

struct UiEntries {
    init: u16,
    frame: u16,
    warm_mark: u16,
    render_token: u16,
    gen_begin: u16,
    gen_end: u16,
    anim_setup: u16,
    anim_restore: u16,
}

// ---------------------------------------------------------------------------
// top-level build
// ---------------------------------------------------------------------------

/// Assemble the complete interactive shell ROM around the stateful forward
/// pass and the pinned integer sampler. `font_tiles` supplies the 76
/// charset glyphs ([`SHELL_FONT_BYTES`] bytes, charset id order);
/// `n_gen_tokens` caps a generation run (1..=[`SHELL_MAX_GEN_TOKENS`]).
pub fn build_state_shell_rom(
    model: &IntStateLoweredModel,
    sampler: &crate::decode::SamplerConfig,
    n_gen_tokens: u8,
    font_tiles: &[u8],
) -> Result<ShellRom, ModelRomError> {
    build_state_shell_rom_lowered(model, sampler, n_gen_tokens, font_tiles, WeightLowering::V3)
}

/// [`build_state_shell_rom`] with an explicit weight lowering. The shell driver
/// (UI + sampler + forward pass) is the largest bank-0 driver, so this is the
/// tightest bank-0 fit check for the V2 shared handler.
pub fn build_state_shell_rom_lowered(
    model: &IntStateLoweredModel,
    sampler: &crate::decode::SamplerConfig,
    n_gen_tokens: u8,
    font_tiles: &[u8],
    lowering: WeightLowering,
) -> Result<ShellRom, ModelRomError> {
    if n_gen_tokens == 0 || n_gen_tokens > SHELL_MAX_GEN_TOKENS {
        return Err(ModelRomError::BadTokenCount {
            n_tokens: u16::from(n_gen_tokens),
        });
    }
    if font_tiles.len() != SHELL_FONT_BYTES {
        return Err(ModelRomError::UiBankOverflow {
            bytes: font_tiles.len(),
        });
    }

    let layout = StateWramLayout::plan(model.topology, model.down_width, true)?;
    let sh = layout
        .shell
        .expect("shell layout allocates the shell block");
    let mut plan = plan_state_rom_with(model, layout, 1, lowering, false)?;
    // Drive the inference animation: `chunk_run` calls `anim_tick` once per
    // weight chunk (SP-safe between chunks). Only the shell enables this.
    plan.animate = true;
    let ui_bank = plan.extras_bank0();
    // Animation frame counter, in the zeroed shell block (free byte prompt+0x2A).
    let anim_fc = sh.prompt + 0x2A;
    let (ui_bytes, ui) = build_ui_bank(font_tiles, &sh)?;
    let ui_bank_bytes = ui_bytes.len();

    let map_ui = |asm: &mut ModelAsm| {
        set_bank(asm, ui_bank as u16);
    };
    let call_abs = |asm: &mut ModelAsm, addr: u16| {
        asm.i(Instr::Call { cond: None, addr });
    };

    // Bank-0 driver: boot -> idle input loop -> warmup -> generation.
    let mut asm = ModelAsm::new(ENTRY_POINT);
    asm.i(Instr::Di);
    ld16(&mut asm, Reg16Data::SP, S_STACK_TOP);
    // zero the shell WRAM block
    ld16(&mut asm, Reg16Data::HL, sh.prompt);
    ld_r_imm(&mut asm, Reg8::B, (sh.end - sh.prompt) as u8);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.label("sh_zero");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "sh_zero");
    // video/UI init
    map_ui(&mut asm);
    call_abs(&mut asm, ui.init);

    // --- idle input loop (one iteration per frame) ---
    asm.label("shell_idle");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.frame);
    a_from(&mut asm, sh.submit);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "shell_idle");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.submit);
    a_from(&mut asm, sh.plen);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "shell_idle"); // ignore empty submits

    // --- generation run ---
    call_abs(&mut asm, ui.gen_begin); // UI bank still mapped
    call_abs(&mut asm, ui.anim_setup); // paint the plasma "thinking" panel
    // zero the recurrent state (trained initial-state contract, fresh
    // context per submit)
    emit_zero16(
        &mut asm,
        plan.layout.state,
        (4 * plan.layout.topology.state_slots) as u16,
    );
    // canonicalize the RNG seed (0 -> 1, decode contract); otherwise the
    // XorShift16 state carries across runs within a session
    a_from(&mut asm, S_RNG_ADDR);
    ld_rr(&mut asm, Reg8::B, Reg8::A);
    a_from(&mut asm, S_RNG_ADDR + 1);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "sh_rng_ok");
    ld_r_imm(&mut asm, Reg8::A, 1);
    a_to(&mut asm, S_RNG_ADDR);
    asm.label("sh_rng_ok");

    // warmup: one forward pass per prompt char, no RNG draws
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.widx);
    asm.label("sh_warm_loop");
    a_from(&mut asm, sh.widx);
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    ld_r_imm(&mut asm, Reg8::H, (sh.prompt >> 8) as u8);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(&mut asm, S_INPUT_ADDR);
    asm.call("forward_pass");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.warm_mark);
    asm.label("shell_warm_boundary");
    a_from(&mut asm, sh.widx);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.widx);
    ld_rr(&mut asm, Reg8::B, Reg8::A);
    a_from(&mut asm, sh.plen);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "sh_warm_loop");

    // sampled generation: sample from the current logits, render, feed back
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.gcount);
    asm.label("sh_gen_loop");
    asm.call("sample_v");
    a_from(&mut asm, sh.gcount);
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    ld_r_imm(&mut asm, Reg8::H, (plan.layout.out >> 8) as u8);
    a_from(&mut asm, S_SAMPLED_ADDR);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    a_to(&mut asm, S_INPUT_ADDR);
    map_ui(&mut asm);
    call_abs(&mut asm, ui.render_token);
    asm.label("shell_token_boundary");
    a_from(&mut asm, sh.gcount);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.gcount);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(n_gen_tokens),
    });
    asm.jp(Some(Cond::Z), "shell_gen_done");
    a_from(&mut asm, sh.tfull);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::NZ), "shell_gen_done");
    asm.call("forward_pass");
    asm.jp(None, "sh_gen_loop");

    asm.label("shell_gen_done");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.anim_restore); // repaint keyboard, reset palette
    call_abs(&mut asm, ui.gen_end);
    asm.jp(None, "shell_idle");

    // the per-token forward pass as a subroutine
    asm.label("forward_pass");
    emit_state_forward_body(&mut asm, &plan);
    asm.i(Instr::Ret { cond: None });

    // `anim_tick`: called by `chunk_run` once per weight chunk (SP home, all
    // registers free). Writes ONLY the BGP register (never VRAM, never waits for
    // VBlank), so it is safe mid-compute. Advance a frame counter and rotate the
    // two plasma shades: BGP = ANIM_BGP_LUT[(FC>>1) & 7]. Text (values 0/3) is
    // untouched by every LUT entry, so it stays black-on-white.
    asm.label("anim_tick");
    a_from(&mut asm, anim_fc);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, anim_fc);
    asm.i(Instr::Srl {
        target: CbTarget::Reg(Reg8::A),
    });
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x07),
    });
    ld_rr(&mut asm, Reg8::E, Reg8::A);
    ld_r_imm(&mut asm, Reg8::D, 0);
    asm.ld16_label(Reg16Data::HL, "anim_bgp", 0);
    asm.i(Instr::AddHl { src: Reg16Data::DE });
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    ldh_a_to(&mut asm, IO_BGP);
    asm.i(Instr::Ret { cond: None });
    asm.label("anim_bgp");
    asm.bytes(ANIM_BGP_LUT.to_vec());

    emit_state_routines_and_tables(&mut asm, model, &plan, Some(sampler));

    let (driver, labels) = asm.finish()?;
    let driver_bytes = driver.len();
    if usize::from(ENTRY_POINT) + driver_bytes > usize::from(CHUNK_ENTRY) {
        return Err(ModelRomError::DriverOverflowsBank0 {
            bytes: driver_bytes,
        });
    }

    let (rom, rom_size, table_bytes) =
        assemble_state_rom("GBFSHELL", driver, &plan, model, &[ui_bytes])?;

    Ok(ShellRom {
        rom,
        layout: plan.layout.clone(),
        idle_pc: labels["shell_idle"],
        warm_boundary_pc: labels["shell_warm_boundary"],
        token_boundary_pc: labels["shell_token_boundary"],
        gen_done_pc: labels["shell_gen_done"],
        n_gen_tokens,
        rom_size,
        bank_count: plan.bank_count as u16,
        driver_bytes,
        ui_bank_bytes,
        weight_code_bytes: plan.weight_code_bytes,
        weight_chunk_count: plan.weight_chunk_count,
        table_bytes,
    })
}

// ===========================================================================
// Subword MoE demo ROM (deploy step 5, `docs/design/integer-moe-deploy.md`):
// the vocab-1024 Paged + 8-expert MoE student generating COHERENT MULTI-CHAR
// subword text on-device, host-byte-identical. The prompt is poked as
// pre-encoded token ids (host `BpeModel::encode`); on-device BPE encode is out
// of scope. Each generated token renders its MULTIPLE literal `id_bytes` to the
// transcript (one token -> several chars), byte-identical to
// `BpeModel::decode`.
// ===========================================================================

/// Byte value the demo renders as a newline (advances the transcript row).
pub const SUBWORD_NEWLINE_BYTE: u8 = b'\n';
/// Byte value the demo renders as a space (blank tile == byte 0x20).
pub const SUBWORD_SPACE_BYTE: u8 = b' ';
/// Demo block-cursor tile: inverted space (space byte + invert offset). The
/// byte-indexed font makes tile 0x20 blank, so 0xA0 is the solid block.
pub const SUBWORD_CURSOR_TILE: u8 = SUBWORD_SPACE_BYTE + SHELL_INVERT_TILE_OFFSET;
/// Number of glyphs in the byte-indexed demo font (ASCII range; tile == byte).
/// The caller supplies [`SUBWORD_FONT_BYTES`] font bytes (tile == byte value);
/// the bench harness builds them from the committed runtime ASCII font.
pub const SUBWORD_FONT_TILES: usize = 128;
pub const SUBWORD_FONT_BYTES: usize = SUBWORD_FONT_TILES * 16;

/// Row layout of the on-device `id_bytes` table: `[len, b0, b1, ...]` padded to
/// `stride`. `stride = next_pow2(1 + max_token_len)` so one row is a
/// power-of-two and the bank/row index is a shift+mask (mirrors the embedding
/// table geometry). Byte b of token id is at ROM offset `id*stride + 1 + b`.
#[derive(Debug, Clone, Copy)]
pub struct IdBytesTableGeometry {
    pub stride: usize,
    pub rows_per_bank: usize,
    pub bank_count: usize,
    pub vocab: usize,
}

impl IdBytesTableGeometry {
    #[must_use]
    pub fn plan(vocab: usize, max_token_len: usize) -> Self {
        let stride = (1 + max_token_len).next_power_of_two().max(2);
        let rows_per_bank = BANK_BYTES / stride;
        let bank_count = vocab.div_ceil(rows_per_bank).max(1);
        Self {
            stride,
            rows_per_bank,
            bank_count,
            vocab,
        }
    }
}

/// Build the `id_bytes` ROM banks from a per-id byte-string table (`id_bytes[i]`
/// = the literal bytes token `i` decodes to, e.g. `BpeModel::id_bytes`). Each
/// bank holds `rows_per_bank` fixed-stride rows; row = `[len, bytes.., 0-pad]`.
#[must_use]
pub fn build_id_bytes_banks(id_bytes: &[Vec<u8>], geom: IdBytesTableGeometry) -> Vec<Vec<u8>> {
    let mut banks = Vec::with_capacity(geom.bank_count);
    for bank_idx in 0..geom.bank_count {
        let lo = bank_idx * geom.rows_per_bank;
        let hi = ((bank_idx + 1) * geom.rows_per_bank).min(geom.vocab);
        let mut bank = Vec::with_capacity((hi - lo) * geom.stride);
        for row in id_bytes.iter().take(hi).skip(lo) {
            let len = row.len().min(geom.stride - 1);
            let before = bank.len();
            bank.push(len as u8);
            bank.extend_from_slice(&row[..len]);
            bank.resize(before + geom.stride, 0);
        }
        banks.push(bank);
    }
    banks
}

/// A fully assembled subword MoE demo ROM plus the trap PCs the runner needs.
#[derive(Debug, Clone)]
pub struct SubwordDemoRom {
    pub rom: Vec<u8>,
    pub layout: StateWramLayout,
    /// Idle head (post-boot / post-run); the driver waits here for the poked
    /// `go` flag before running the poked prompt.
    pub idle_pc: u16,
    /// Hit once per warmup prompt token after its forward pass.
    pub warm_boundary_pc: u16,
    /// Hit once per generated token after its multi-char render.
    pub token_boundary_pc: u16,
    /// Hit once when a generation run completes.
    pub gen_done_pc: u16,
    pub n_gen_tokens: u8,
    /// WRAM base of the poked prompt-token-id buffer (u16 LE per id).
    pub prompt_ids_addr: u16,
    /// WRAM byte holding the poked prompt length (number of u16 ids).
    pub prompt_len_addr: u16,
    /// WRAM byte the host pokes to 1 to start a run (the demo "START").
    pub go_addr: u16,
    pub id_bytes_geom: IdBytesTableGeometry,
    pub rom_size: RomSize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub ui_bank_bytes: usize,
    pub table_bytes: usize,
}

/// `ui_render_bytes` (UI-render bank): render the picked u16 token id's literal
/// bytes to the transcript. The id_bytes bank is mapped by the caller; A holds
/// the row's low byte, the routine reads `len` then each byte, painting
/// `tile == byte` at the transcript cursor (a newline byte advances the row,
/// exactly like the host `expected_subword_transcript_bg`). Sets `sh.tfull`
/// when the region fills. HL points at the row start on entry.
fn emit_ui_render_bytes(asm: &mut ModelAsm, sh: &ShellWram, bcur: u16) {
    use gbf_asm::isa::{IncDec8Target, Reg16Addr};
    asm.label("ui_render_bytes");
    // LCD off around the transcript writes (a handful of BG cells per token):
    // no VBlank sync needed, and it keeps the fast emulator run cheap while
    // staying glitch-free on real hardware. Restored at `urb_done`. HL (the row
    // pointer) must survive, so stash/restore A only.
    ld_rr(asm, Reg8::D, Reg8::H);
    ld_rr(asm, Reg8::E, Reg8::L);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC);
    ld_rr(asm, Reg8::H, Reg8::D);
    ld_rr(asm, Reg8::L, Reg8::E);
    // B = len := (HL++) ; if 0, nothing to draw
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "urb_ret");
    ld_rr(asm, Reg8::B, Reg8::A);
    // Save the row pointer (HL) in DE across the per-byte cell writes.
    ld_rr(asm, Reg8::D, Reg8::H);
    ld_rr(asm, Reg8::E, Reg8::L);
    asm.label("urb_byte");
    // stop if the transcript region is already full
    a_from(asm, sh.tfull);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "urb_ret");
    // byte := (DE++) ; save B (remaining count) and DE (row cursor)
    ld_rr(asm, Reg8::H, Reg8::D);
    ld_rr(asm, Reg8::L, Reg8::E);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::D, Reg8::H);
    ld_rr(asm, Reg8::E, Reg8::L);
    a_to(asm, bcur); // stash the current byte
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(SUBWORD_NEWLINE_BYTE),
    });
    asm.jr(Some(Cond::Z), "urb_nl");
    // glyph write: tile == byte at the transcript cursor, advance one cell
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::Push {
        src: gbf_asm::isa::Reg16Stack::DE,
    });
    ld_rr(asm, Reg8::D, Reg8::A); // D = remaining count (ui_cell_addr clobbers B)
    a_from(asm, bcur);
    ld_rr(asm, Reg8::E, Reg8::A); // E = glyph tile (== byte)
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    a_from(asm, sh.tcur);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.tcur);
    ld_rr(asm, Reg8::B, Reg8::D); // restore remaining count
    asm.i(Instr::Pop {
        dst: gbf_asm::isa::Reg16Stack::DE,
    });
    asm.jr(None, "urb_advance");
    asm.label("urb_nl");
    // newline: erase the block cursor at the current cell, advance to next row
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::Push {
        src: gbf_asm::isa::Reg16Stack::DE,
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SUBWORD_SPACE_BYTE,
    });
    // new cell = (row + 1) * 20  (mirror ui_render_token's row math)
    a_from(asm, sh.tcur);
    ld_r_imm(asm, Reg8::B, 0);
    asm.label("urb_div");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.jr(Some(Cond::C), "urb_dd");
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(TRANSCRIPT_COLS),
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, "urb_div");
    asm.label("urb_dd");
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    }); // *5
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // *20
    a_to(asm, sh.tcur);
    ld_rr(asm, Reg8::B, Reg8::D); // restore remaining count
    asm.i(Instr::Pop {
        dst: gbf_asm::isa::Reg16Stack::DE,
    });
    asm.label("urb_advance");
    // if tcur >= TRANSCRIPT_CELLS, mark full; else keep looping bytes
    a_from(asm, sh.tcur);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_CELLS),
    });
    asm.jr(Some(Cond::C), "urb_next");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, sh.tfull);
    asm.label("urb_next");
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "urb_byte");
    asm.label("urb_ret");
    // draw the block cursor at the current cell unless the region filled
    a_from(asm, sh.tfull);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "urb_done");
    a_from(asm, sh.tcur);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SUBWORD_CURSOR_TILE,
    });
    asm.label("urb_done");
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC); // LCD back on
    asm.i(Instr::Ret { cond: None });
}

/// `dui_init` (subword demo UI bank): LCD off, palette/scroll, upload the
/// byte-indexed font (normal at 0x8000, inverted at 0x8800), clear the whole BG
/// map to the space byte, LCD on. No keyboard / status chrome — the demo pokes
/// its prompt as token ids, so the screen is just the transcript.
fn emit_dui_init(asm: &mut ModelAsm) {
    use gbf_asm::isa::Reg16Addr;
    asm.label("dui_init");
    // LCD off immediately for the bulk VRAM upload (no VBlank wait needed).
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC);
    ldh_a_to(asm, IO_SCY);
    ldh_a_to(asm, IO_SCX);
    ld_r_imm(asm, Reg8::A, BGP_STANDARD);
    ldh_a_to(asm, IO_BGP);
    // font: normal tiles at 0x8000
    asm.ld16_label(Reg16Data::HL, "subword_font", 0);
    ld16(asm, Reg16Data::DE, 0x8000);
    ld16(asm, Reg16Data::BC, SUBWORD_FONT_BYTES as u16);
    asm.label("dui_fc1");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "dui_fc1");
    // font: inverted tiles at 0x8800 (complement both planes)
    asm.ld16_label(Reg16Data::HL, "subword_font", 0);
    ld16(asm, Reg16Data::DE, 0x8800);
    ld16(asm, Reg16Data::BC, SUBWORD_FONT_BYTES as u16);
    asm.label("dui_fc2");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::Cpl);
    asm.i(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "dui_fc2");
    // clear the whole 32x32 BG map to the space byte
    ld16(asm, Reg16Data::HL, BG_MAP_BASE);
    ld16(asm, Reg16Data::BC, 1024);
    asm.label("dui_cl");
    ld_r_imm(asm, Reg8::A, SUBWORD_SPACE_BYTE);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "dui_cl");
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC);
    asm.i(Instr::Ret { cond: None });
}

/// `dui_gen_begin` (subword demo): clear the transcript region, reset the
/// transcript cursor/full flag, draw the block cursor at cell 0. Mirrors
/// `ui_gen_begin` minus the "GENERATING" message row. LCD off for the bulk
/// transcript clear (one batched VRAM write, then LCD back on) instead of a
/// per-row VBlank wait — the transcript spans two BG-map segments (rows 0..9 at
/// stride 32) so a single LCD-off clear is both correct and fast.
fn emit_dui_gen_begin(asm: &mut ModelAsm, sh: &ShellWram) {
    use gbf_asm::isa::{IncDec8Target, Reg16Addr};
    asm.label("dui_gen_begin");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ldh_a_to(asm, IO_LCDC); // LCD off for the bulk clear (no VBlank wait needed)
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.ui_row);
    asm.label("dgb_row");
    a_from(asm, sh.ui_row);
    for _ in 0..4 {
        asm.i(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // row*32 low, CF = bit 8
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (BG_MAP_BASE >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_r_imm(asm, Reg8::B, TRANSCRIPT_COLS);
    ld_r_imm(asm, Reg8::A, SUBWORD_SPACE_BYTE);
    asm.label("dgb_fill");
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "dgb_fill");
    a_from(asm, sh.ui_row);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, sh.ui_row);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_ROWS),
    });
    asm.jp(Some(Cond::NZ), "dgb_row");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, sh.tcur);
    a_to(asm, sh.tfull);
    ld16(asm, Reg16Data::HL, BG_MAP_BASE);
    asm.i(Instr::Ld8HlFromImm {
        imm: SUBWORD_CURSOR_TILE,
    });
    ld_r_imm(asm, Reg8::A, LCDC_ON);
    ldh_a_to(asm, IO_LCDC); // LCD back on
    asm.i(Instr::Ret { cond: None });
}

struct DemoUiEntries {
    init: u16,
    gen_begin: u16,
    render_bytes: u16,
}

/// Build the subword-demo UI bank (render routines + byte-indexed font). The
/// `id_bytes` table lives in SEPARATE data banks; this bank is code + font.
fn build_subword_ui_bank(
    font_tiles: &[u8],
    sh: &ShellWram,
    bcur: u16,
) -> Result<(Vec<u8>, DemoUiEntries), ModelRomError> {
    debug_assert_eq!(font_tiles.len(), SUBWORD_FONT_BYTES);
    let mut asm = ModelAsm::new(CHUNK_ENTRY);
    emit_dui_init(&mut asm);
    emit_dui_gen_begin(&mut asm, sh);
    emit_ui_render_bytes(&mut asm, sh, bcur);
    emit_ui_cell_addr(&mut asm);
    asm.label("subword_font");
    asm.bytes(font_tiles.to_vec());
    let (bytes, labels) = asm.finish()?;
    if bytes.len() > BANK_BYTES {
        return Err(ModelRomError::UiBankOverflow { bytes: bytes.len() });
    }
    let entries = DemoUiEntries {
        init: labels["dui_init"],
        gen_begin: labels["dui_gen_begin"],
        render_bytes: labels["ui_render_bytes"],
    };
    Ok((bytes, entries))
}

/// Build the subword MoE demo ROM (deploy step 5): the vocab-1024 Paged +
/// `n_experts`-way MoE student generating multi-char subword text on-device. The
/// prompt is poked as pre-encoded u16 token ids (`prompt_ids_addr` /
/// `prompt_len_addr`); setting `go_addr` to 1 runs one generation: warm up over
/// the poked prompt ids, then sample `n_gen_tokens` tokens from the paged head,
/// feeding the FULL u16 id back through the embedding lookup and rendering each
/// token's literal `id_bytes` (multiple chars) to the transcript.
///
/// Requires a Paged wide-vocab MoE topology (`is_moe()` and `LogitPaging::Paged`)
/// and V2 dispatch (one expert resident per token). `font_tiles` is the
/// byte-indexed [`SUBWORD_FONT_BYTES`] font (tile == byte). `id_bytes[i]` is the
/// literal byte string token `i` decodes to (`BpeModel::id_bytes`).
#[allow(clippy::too_many_lines)]
pub fn build_state_moe_demo_rom(
    model: &IntStateLoweredModel,
    sampler: &crate::decode::SamplerConfig,
    n_gen_tokens: u8,
    font_tiles: &[u8],
    id_bytes: &[Vec<u8>],
) -> Result<SubwordDemoRom, ModelRomError> {
    use crate::asm_impl_state::{S_INPUT_HI_ADDR, S_SAMPLED_HI_ADDR};
    if n_gen_tokens == 0 || n_gen_tokens > SHELL_MAX_GEN_TOKENS {
        return Err(ModelRomError::BadTokenCount {
            n_tokens: u16::from(n_gen_tokens),
        });
    }
    if font_tiles.len() != SUBWORD_FONT_BYTES {
        return Err(ModelRomError::UiBankOverflow {
            bytes: font_tiles.len(),
        });
    }
    let t = model.topology;
    if !t.is_moe() || t.logit_paging != crate::state_model_ref::LogitPaging::Paged {
        return Err(ModelRomError::BadTokenCount {
            n_tokens: u16::from(n_gen_tokens),
        });
    }

    // id_bytes table geometry (fixed-stride rows across data banks).
    let max_len = id_bytes.iter().map(Vec::len).max().unwrap_or(1);
    let geom = IdBytesTableGeometry::plan(t.vocab, max_len);
    let id_bytes_banks = build_id_bytes_banks(id_bytes, geom);

    let layout = StateWramLayout::plan(model.topology, model.down_width, true)?;
    let sh = layout
        .shell
        .expect("shell layout allocates the shell block");
    // extra banks: 1 UI (code + font) + the id_bytes data banks.
    let extra_banks = 1 + geom.bank_count;
    // The real subword MoE demo is the one ROM whose bank-0 driver (shell +
    // paged sampler + subword render + MoE dispatch) overflows the 16 KiB
    // window. Relocate the fully-unrolled `isqrt48` (~4.8 KiB) into its own
    // switchable bank to reclaim the space; every other ROM keeps it in bank 0
    // (byte-identical driver + bank numbering).
    let plan = plan_state_rom_with(model, layout, extra_banks, WeightLowering::V2Dispatch, true)?;
    let ui_bank = plan.extras_bank0();
    let id_bytes_bank0 = ui_bank + 1;

    // WRAM reuse of the shell 0x100 page: the host pokes the prompt-id buffer
    // (u16 LE, +0x40..+0xC0 = 64 ids), the demo copies each token's id_bytes row
    // into `row_buf` (+0xC0..+0x100), and `go`/`plen`/cursors reuse the control
    // bytes. `bcur` is a per-byte render scratch. The stride must fit `row_buf`.
    let prompt_ids_addr = sh.prompt + 0x40;
    let row_buf = sh.prompt + 0xC0;
    let go_addr = sh.submit;
    let plen_addr = sh.plen;
    let bcur = sh.prompt + 0x2B; // free byte (anim_fc uses +0x2A; unused here)
    if geom.stride > 0x40 {
        return Err(ModelRomError::TableRowTooWide {
            stride: geom.stride,
        });
    }
    // 64-id prompt cap (u16 buffer); more would collide with `row_buf`.
    if n_gen_tokens == 0 {
        return Err(ModelRomError::BadTokenCount { n_tokens: 0 });
    }

    let (ui_bytes, ui) = build_subword_ui_bank(font_tiles, &sh, bcur)?;
    let ui_bank_bytes = ui_bytes.len();

    let id_bytes_geom_log_rpb = geom.rows_per_bank.trailing_zeros();
    let id_bytes_log_stride = geom.stride.trailing_zeros();
    // Full 16-bit row-within-bank mask (rows_per_bank can exceed 256).
    let id_bytes_row_mask_lo = ((geom.rows_per_bank - 1) & 0xFF) as u8;
    let id_bytes_row_mask_hi = (((geom.rows_per_bank - 1) >> 8) & 0xFF) as u8;

    let map_ui = |asm: &mut ModelAsm| set_bank(asm, ui_bank as u16);
    let call_abs = |asm: &mut ModelAsm, addr: u16| {
        asm.i(Instr::Call { cond: None, addr });
    };

    // --- bank-0 driver ---
    let mut asm = ModelAsm::new(ENTRY_POINT);
    asm.i(Instr::Di);
    ld16(&mut asm, Reg16Data::SP, S_STACK_TOP);
    // zero the shell control block (go/plen/cursors); the prompt-id buffer is
    // host-poked, so it lives above `sh.end` and is not cleared here.
    ld16(&mut asm, Reg16Data::HL, sh.prompt);
    ld_r_imm(&mut asm, Reg8::B, (sh.end - sh.prompt) as u8);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.label("dsh_zero");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "dsh_zero");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.init);

    // --- idle: poll for the poked `go` flag (tight loop; no VBlank wait — the
    // screen is static while idle, so no VRAM writes need syncing) ---
    asm.label("demo_idle");
    a_from(&mut asm, go_addr);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "demo_idle");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, go_addr);
    a_from(&mut asm, plen_addr);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "demo_idle"); // ignore empty prompts

    // --- generation run ---
    map_ui(&mut asm);
    call_abs(&mut asm, ui.gen_begin);
    // zero the recurrent state (fresh context per submit)
    emit_zero16(
        &mut asm,
        plan.layout.state,
        (4 * plan.layout.topology.state_slots) as u16,
    );
    // canonicalize the RNG seed (0 -> 1)
    a_from(&mut asm, S_RNG_ADDR);
    ld_rr(&mut asm, Reg8::B, Reg8::A);
    a_from(&mut asm, S_RNG_ADDR + 1);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "dsh_rng_ok");
    ld_r_imm(&mut asm, Reg8::A, 1);
    a_to(&mut asm, S_RNG_ADDR);
    asm.label("dsh_rng_ok");

    // --- warmup: one forward pass per poked prompt token id (no RNG draws) ---
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.widx);
    asm.label("dsh_warm_loop");
    // load prompt_ids[widx] (u16 LE) -> S_INPUT / S_INPUT_HI
    a_from(&mut asm, sh.widx);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // *2
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((prompt_ids_addr & 0xFF) as u8),
    });
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    ld_r_imm(&mut asm, Reg8::A, (prompt_ids_addr >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(&mut asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    a_to(&mut asm, S_INPUT_ADDR);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(&mut asm, S_INPUT_HI_ADDR);
    asm.call("forward_pass");
    asm.label("demo_warm_boundary");
    a_from(&mut asm, sh.widx);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.widx);
    ld_rr(&mut asm, Reg8::B, Reg8::A);
    a_from(&mut asm, plen_addr);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "dsh_warm_loop");

    // --- generation: sample (paged), render id_bytes, feed the full id back ---
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.gcount);
    asm.label("dsh_gen_loop");
    asm.call("sample_paged");
    // feed the full picked id back: S_SAMPLED/HI -> S_INPUT/HI
    a_from(&mut asm, S_SAMPLED_ADDR);
    a_to(&mut asm, S_INPUT_ADDR);
    a_from(&mut asm, S_SAMPLED_HI_ADDR);
    a_to(&mut asm, S_INPUT_HI_ADDR);
    // render: the id_bytes row and the render CODE live in DIFFERENT switchable
    // banks, so copy the row into WRAM (`row_buf`) while the id_bytes bank is
    // mapped, THEN map the UI-render bank and paint from WRAM.
    // id = S_SAMPLED_HI:S_SAMPLED ; bank = id_bytes_bank0 + (id >> log_rpb)
    a_from(&mut asm, S_SAMPLED_HI_ADDR);
    ld_rr(&mut asm, Reg8::D, Reg8::A);
    a_from(&mut asm, S_SAMPLED_ADDR);
    for _ in 0..id_bytes_geom_log_rpb {
        asm.i(Instr::Srl {
            target: CbTarget::Reg(Reg8::D),
        });
        asm.i(Instr::Rr {
            target: CbTarget::Reg(Reg8::A),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((id_bytes_bank0 & 0xFF) as u8),
    });
    a_to(&mut asm, MBC5_ROMB0);
    ld_r_imm(&mut asm, Reg8::A, (id_bytes_bank0 >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(&mut asm, MBC5_ROMB1);
    // HL = CHUNK_ENTRY + ((id & (rows_per_bank-1)) << log_stride)  (row within
    // bank). The row index is a FULL 16-bit value (ids >= 256), so mask both
    // bytes of the id then 16-bit shift-left by log_stride.
    a_from(&mut asm, S_SAMPLED_ADDR);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(id_bytes_row_mask_lo),
    });
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    a_from(&mut asm, S_SAMPLED_HI_ADDR);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(id_bytes_row_mask_hi),
    });
    ld_rr(&mut asm, Reg8::H, Reg8::A);
    for _ in 0..id_bytes_log_stride {
        asm.i(Instr::AddHl { src: Reg16Data::HL });
    }
    ld_rr(&mut asm, Reg8::A, Reg8::H);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(&mut asm, Reg8::H, Reg8::A);
    // copy `stride` bytes: (HL) -> row_buf
    ld16(&mut asm, Reg16Data::DE, row_buf);
    ld_r_imm(&mut asm, Reg8::B, geom.stride as u8);
    asm.label("dsh_rowcopy");
    asm.i(Instr::LdAFromReg16Addr {
        src: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::DE,
    });
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "dsh_rowcopy");
    // switch to the UI-render bank and paint the row's bytes (HL = row_buf)
    ld16(&mut asm, Reg16Data::HL, row_buf);
    map_ui(&mut asm);
    call_abs(&mut asm, ui.render_bytes);
    asm.label("demo_token_boundary");
    a_from(&mut asm, sh.gcount);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, sh.gcount);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(n_gen_tokens),
    });
    asm.jp(Some(Cond::Z), "demo_gen_done");
    a_from(&mut asm, sh.tfull);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::NZ), "demo_gen_done");
    asm.call("forward_pass");
    asm.jp(None, "dsh_gen_loop");

    asm.label("demo_gen_done");
    asm.jp(None, "demo_idle");

    // per-token forward pass subroutine (paged head; wide emb feedback)
    asm.label("forward_pass");
    emit_state_forward_body(&mut asm, &plan);
    asm.i(Instr::Ret { cond: None });

    emit_state_routines_and_tables(&mut asm, model, &plan, Some(sampler));

    let (driver, labels) = asm.finish()?;
    let driver_bytes = driver.len();
    if usize::from(ENTRY_POINT) + driver_bytes > usize::from(CHUNK_ENTRY) {
        return Err(ModelRomError::DriverOverflowsBank0 {
            bytes: driver_bytes,
        });
    }

    let mut extra_payloads = Vec::with_capacity(extra_banks);
    extra_payloads.push(ui_bytes);
    extra_payloads.extend(id_bytes_banks);
    let (rom, rom_size, table_bytes) =
        assemble_state_rom("GBFMOEDEMO", driver, &plan, model, &extra_payloads)?;

    Ok(SubwordDemoRom {
        rom,
        layout: plan.layout.clone(),
        idle_pc: labels["demo_idle"],
        warm_boundary_pc: labels["demo_warm_boundary"],
        token_boundary_pc: labels["demo_token_boundary"],
        gen_done_pc: labels["demo_gen_done"],
        n_gen_tokens,
        prompt_ids_addr,
        prompt_len_addr: plen_addr,
        go_addr,
        id_bytes_geom: geom,
        rom_size,
        bank_count: plan.bank_count as u16,
        driver_bytes,
        ui_bank_bytes,
        table_bytes,
    })
}

/// A minimal programmatic fallback font for tests: each glyph is a
/// distinctive 2bpp pattern derived from its id (visibly distinct, and
/// byte-distinct per id so BG assertions cannot alias).
#[must_use]
pub fn synthetic_font_tiles() -> Vec<u8> {
    let mut font = Vec::with_capacity(SHELL_FONT_BYTES);
    for id in 0..SHELL_FONT_TILES as u8 {
        for row in 0..8u8 {
            let v = if id == SHELL_SPACE_ID {
                0
            } else {
                (id ^ (row.wrapping_mul(37))) | 0x01
            };
            font.push(v);
            font.push(v);
        }
    }
    font
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asm_impl_state::build_state_multi_token_sampling_rom;
    use crate::state_model_ref::synthetic_state_checkpoint;

    #[test]
    fn shell_rom_builds_from_synthetic_checkpoint() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = crate::decode::SamplerConfig::new(8, 2253).expect("valid sampler");
        let font = synthetic_font_tiles();
        let rom = build_state_shell_rom(&lowered, &cfg, 200, &font).expect("builds");
        assert_eq!(rom.rom.len(), rom.rom_size.bytes());
        assert_eq!(rom.rom[0x0147], 0x19, "MBC5 cartridge type");
        // one extra bank beyond the sampling ROM (the UI bank)
        let sampling = build_state_multi_token_sampling_rom(&lowered, 16, &cfg).expect("builds");
        assert_eq!(rom.bank_count, sampling.bank_count + 1);
        assert_eq!(rom.weight_chunk_count, sampling.weight_chunk_count);
        assert_eq!(rom.table_bytes, sampling.table_bytes);
        assert!(rom.ui_bank_bytes > SHELL_FONT_BYTES);
        assert!(rom.ui_bank_bytes <= BANK_BYTES);
        // trap PCs are distinct bank-0 addresses
        for pc in [
            rom.idle_pc,
            rom.warm_boundary_pc,
            rom.token_boundary_pc,
            rom.gen_done_pc,
        ] {
            assert!((ENTRY_POINT..CHUNK_ENTRY).contains(&pc), "pc {pc:#06x}");
        }
        assert!(rom.idle_pc < rom.warm_boundary_pc);
        assert!(rom.warm_boundary_pc < rom.token_boundary_pc);
        assert!(rom.token_boundary_pc < rom.gen_done_pc);
    }

    #[test]
    fn shell_rom_rejects_bad_token_counts_and_fonts() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = crate::decode::SamplerConfig::new(8, 2253).expect("valid sampler");
        let font = synthetic_font_tiles();
        assert!(matches!(
            build_state_shell_rom(&lowered, &cfg, 0, &font),
            Err(ModelRomError::BadTokenCount { n_tokens: 0 })
        ));
        assert!(matches!(
            build_state_shell_rom(&lowered, &cfg, 201, &font),
            Err(ModelRomError::BadTokenCount { n_tokens: 201 })
        ));
        assert!(matches!(
            build_state_shell_rom(&lowered, &cfg, 8, &font[..100]),
            Err(ModelRomError::UiBankOverflow { .. })
        ));
    }

    #[test]
    fn status_and_message_ids_are_charset_printables() {
        for id in SHELL_STATUS_TEXT_IDS
            .iter()
            .chain(SHELL_MSG_TEXT_IDS.iter())
        {
            assert!(*id < 76, "id {id} outside charset_v1");
            assert_ne!(*id, SHELL_NEWLINE_ID);
        }
    }
}
