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
    BANK_BYTES, CHUNK_ENTRY, MBC5_ROMB0, ModelAsm, ModelRomError, a_from, a_to, ld_r_imm, ld_rr,
    ld16,
};
use crate::asm_impl_state::{
    S_INPUT_ADDR, S_OUT_BASE, S_RNG_ADDR, S_SAMPLED_ADDR, S_STACK_TOP, S_STATE_BASE,
    assemble_state_rom, emit_state_forward_body, emit_state_routines_and_tables, plan_state_rom,
};
use crate::state_model_ref::IntStateLoweredModel;

// ---------------------------------------------------------------------------
// WRAM map (shell-owned block; disjoint from every stateful-ROM buffer)
// ---------------------------------------------------------------------------

/// Prompt buffer (charset ids), [`SHELL_PROMPT_CAP`] bytes. The base is
/// page-aligned so `lo(addr) == index`.
pub const SH_PROMPT_BASE: u16 = 0xD400;
/// Maximum prompt length (one BG row).
pub const SHELL_PROMPT_CAP: u8 = 20;
/// Current prompt length.
pub const SH_PLEN_ADDR: u16 = 0xD418;
/// Submit flag (set by START, consumed by the control loop).
pub const SH_SUBMIT_ADDR: u16 = 0xD419;
/// Keyboard cursor cell index (0..=75; cell index == charset id).
pub const SH_KBCUR_ADDR: u16 = 0xD41A;
/// Joypad state, active-high (this frame / previous frame).
pub const SH_JOY_CUR_ADDR: u16 = 0xD41B;
pub const SH_JOY_PREV_ADDR: u16 = 0xD41C;
/// Warmup index over the prompt.
pub const SH_WIDX_ADDR: u16 = 0xD41D;
/// Tokens generated in the current run.
pub const SH_GCOUNT_ADDR: u16 = 0xD41E;
/// Transcript cell cursor (0..=200).
pub const SH_TCUR_ADDR: u16 = 0xD41F;
/// Set when the transcript region filled (ends the run).
pub const SH_TFULL_ADDR: u16 = 0xD420;
/// UI scratch (row counter for multi-VBlank clears).
pub const SH_UI_ROW_ADDR: u16 = 0xD421;
/// End of the zero-initialized shell WRAM block `[SH_PROMPT_BASE, ..)`.
pub const SH_WRAM_END: u16 = 0xD430;

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

/// A fully assembled interactive shell ROM plus the facts and trap PCs the
/// runner needs.
#[derive(Debug, Clone)]
pub struct ShellRom {
    pub rom: Vec<u8>,
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
fn emit_ui_joypad(asm: &mut ModelAsm) {
    asm.label("ui_joypad");
    a_from(asm, SH_JOY_CUR_ADDR);
    a_to(asm, SH_JOY_PREV_ADDR);
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
    a_to(asm, SH_JOY_CUR_ADDR);
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
fn emit_ui_kb_move(asm: &mut ModelAsm) {
    asm.label("ui_kb_move");
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, SH_KBCUR_ADDR);
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
fn emit_ui_frame(asm: &mut ModelAsm) {
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE; // 0x9960
    asm.label("ui_frame");
    asm.call("ui_wait_vbl");
    asm.call("ui_joypad");
    // D = newly pressed = CUR & !PREV
    a_from(asm, SH_JOY_PREV_ADDR);
    asm.i(Instr::Cpl);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SH_JOY_CUR_ADDR);
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
    a_from(asm, SH_KBCUR_ADDR);
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
    a_from(asm, SH_KBCUR_ADDR);
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
    a_from(asm, SH_KBCUR_ADDR);
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
    a_from(asm, SH_KBCUR_ADDR);
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
    a_from(asm, SH_PLEN_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(SHELL_PROMPT_CAP),
    });
    asm.jr(Some(Cond::NC), "uf_no_a");
    ld_rr(asm, Reg8::C, Reg8::A); // C = plen
    a_from(asm, SH_KBCUR_ADDR);
    ld_rr(asm, Reg8::E, Reg8::A); // E = id
    // prompt[plen] = id  (SH_PROMPT_BASE low byte is 0x00)
    ld_r_imm(asm, Reg8::H, (SH_PROMPT_BASE >> 8) as u8);
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
    a_to(asm, SH_PLEN_ADDR);
    asm.label("uf_no_a");

    // B (bit 1): backspace
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x02),
    });
    asm.jr(Some(Cond::Z), "uf_no_b");
    a_from(asm, SH_PLEN_ADDR);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "uf_no_b");
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, SH_PLEN_ADDR);
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
    a_to(asm, SH_SUBMIT_ADDR);
    asm.label("uf_no_s");
    asm.i(Instr::Ret { cond: None });
}

/// `ui_warm_mark`: highlight (invert) prompt char [`SH_WIDX_ADDR`] on the
/// prompt row — the warmup progress affordance.
fn emit_ui_warm_mark(asm: &mut ModelAsm) {
    let prompt_bg = BG_MAP_BASE + u16::from(PROMPT_ROW) * BG_MAP_STRIDE;
    asm.label("ui_warm_mark");
    asm.call("ui_wait_vbl");
    a_from(asm, SH_WIDX_ADDR);
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::H, (SH_PROMPT_BASE >> 8) as u8);
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
/// now full, [`SH_TFULL_ADDR`] is set; otherwise the block cursor is drawn
/// at the new cell.
fn emit_ui_render_token(asm: &mut ModelAsm) {
    asm.label("ui_render_token");
    asm.call("ui_wait_vbl");
    a_from(asm, S_SAMPLED_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(SHELL_NEWLINE_ID),
    });
    asm.jr(Some(Cond::Z), "urt_nl");
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, SH_TCUR_ADDR);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromReg { src: Reg8::E });
    a_from(asm, SH_TCUR_ADDR);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, SH_TCUR_ADDR);
    asm.jr(None, "urt_after");
    asm.label("urt_nl");
    a_from(asm, SH_TCUR_ADDR);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_SPACE_ID,
    });
    // new cell = (row + 1) * 20
    a_from(asm, SH_TCUR_ADDR);
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
    a_to(asm, SH_TCUR_ADDR);
    asm.label("urt_after");
    a_from(asm, SH_TCUR_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_CELLS),
    });
    asm.jr(Some(Cond::C), "urt_nf");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SH_TFULL_ADDR);
    asm.i(Instr::Ret { cond: None });
    asm.label("urt_nf");
    a_from(asm, SH_TCUR_ADDR);
    asm.call("ui_cell_addr");
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_CURSOR_TILE,
    });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_gen_begin`: show the "GENERATING" message, clear the transcript
/// region (one row per VBlank), reset the transcript cursor, and draw the
/// block cursor at cell 0.
fn emit_ui_gen_begin(asm: &mut ModelAsm) {
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
    a_to(asm, SH_UI_ROW_ADDR);
    asm.label("ugb_row");
    asm.call("ui_wait_vbl");
    a_from(asm, SH_UI_ROW_ADDR);
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
    a_from(asm, SH_UI_ROW_ADDR);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, SH_UI_ROW_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(TRANSCRIPT_ROWS),
    });
    asm.jp(Some(Cond::NZ), "ugb_row");
    // reset transcript state, draw the block cursor at cell 0
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, SH_TCUR_ADDR);
    a_to(asm, SH_TFULL_ADDR);
    asm.call("ui_wait_vbl");
    ld16(asm, Reg16Data::HL, BG_MAP_BASE);
    asm.i(Instr::Ld8HlFromImm {
        imm: SHELL_CURSOR_TILE,
    });
    asm.i(Instr::Ret { cond: None });
}

/// `ui_gen_end`: clear the "GENERATING" message and the prompt row, and
/// reset the prompt length for the next entry.
fn emit_ui_gen_end(asm: &mut ModelAsm) {
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
    a_to(asm, SH_PLEN_ADDR);
    asm.i(Instr::Ret { cond: None });
}

/// `ui_init`: LCD off (from inside VBlank), palette/scroll setup, font tile
/// upload (normal to 0x8000, inverted to 0x8800), full BG-map clear,
/// keyboard grid + status row + initial cursor, then LCD on.
fn emit_ui_init(asm: &mut ModelAsm) {
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
    a_to(asm, SH_KBCUR_ADDR);
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

/// Build the UI bank image (routines + font + text data) and return its
/// bytes plus the entry addresses bank-0 code calls.
fn build_ui_bank(font_tiles: &[u8]) -> Result<(Vec<u8>, UiEntries), ModelRomError> {
    debug_assert_eq!(font_tiles.len(), SHELL_FONT_BYTES);
    let mut asm = ModelAsm::new(CHUNK_ENTRY);
    emit_ui_init(&mut asm);
    emit_ui_frame(&mut asm);
    emit_ui_warm_mark(&mut asm);
    emit_ui_render_token(&mut asm);
    emit_ui_gen_begin(&mut asm);
    emit_ui_gen_end(&mut asm);
    emit_ui_wait_vbl(&mut asm);
    emit_ui_joypad(&mut asm);
    emit_ui_kb_addr(&mut asm);
    emit_ui_cell_addr(&mut asm);
    emit_ui_kb_move(&mut asm);
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

    let plan = plan_state_rom(model, 1)?;
    let ui_bank = plan.head_bank + 1;
    let ui_bank_u8 = ui_bank as u8;
    let (ui_bytes, ui) = build_ui_bank(font_tiles)?;
    let ui_bank_bytes = ui_bytes.len();

    let map_ui = |asm: &mut ModelAsm| {
        ld_r_imm(asm, Reg8::A, ui_bank_u8);
        a_to(asm, MBC5_ROMB0);
    };
    let call_abs = |asm: &mut ModelAsm, addr: u16| {
        asm.i(Instr::Call { cond: None, addr });
    };

    // Bank-0 driver: boot -> idle input loop -> warmup -> generation.
    let mut asm = ModelAsm::new(ENTRY_POINT);
    asm.i(Instr::Di);
    ld16(&mut asm, Reg16Data::SP, S_STACK_TOP);
    // zero the shell WRAM block
    ld16(&mut asm, Reg16Data::HL, SH_PROMPT_BASE);
    ld_r_imm(&mut asm, Reg8::B, (SH_WRAM_END - SH_PROMPT_BASE) as u8);
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
    a_from(&mut asm, SH_SUBMIT_ADDR);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "shell_idle");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, SH_SUBMIT_ADDR);
    a_from(&mut asm, SH_PLEN_ADDR);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "shell_idle"); // ignore empty submits

    // --- generation run ---
    call_abs(&mut asm, ui.gen_begin); // UI bank still mapped
    // zero the recurrent state (256 bytes; trained initial-state contract,
    // fresh context per submit)
    ld16(&mut asm, Reg16Data::HL, S_STATE_BASE);
    ld_r_imm(&mut asm, Reg8::B, 0);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.label("sh_zstate");
    asm.i(Instr::LdReg16AddrFromA {
        dst: gbf_asm::isa::Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "sh_zstate");
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
    a_to(&mut asm, SH_WIDX_ADDR);
    asm.label("sh_warm_loop");
    a_from(&mut asm, SH_WIDX_ADDR);
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    ld_r_imm(&mut asm, Reg8::H, (SH_PROMPT_BASE >> 8) as u8);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(&mut asm, S_INPUT_ADDR);
    asm.call("forward_pass");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.warm_mark);
    asm.label("shell_warm_boundary");
    a_from(&mut asm, SH_WIDX_ADDR);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, SH_WIDX_ADDR);
    ld_rr(&mut asm, Reg8::B, Reg8::A);
    a_from(&mut asm, SH_PLEN_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "sh_warm_loop");

    // sampled generation: sample from the current logits, render, feed back
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(&mut asm, SH_GCOUNT_ADDR);
    asm.label("sh_gen_loop");
    asm.call("sample80");
    a_from(&mut asm, SH_GCOUNT_ADDR);
    ld_rr(&mut asm, Reg8::L, Reg8::A);
    ld_r_imm(&mut asm, Reg8::H, (S_OUT_BASE >> 8) as u8);
    a_from(&mut asm, S_SAMPLED_ADDR);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    a_to(&mut asm, S_INPUT_ADDR);
    map_ui(&mut asm);
    call_abs(&mut asm, ui.render_token);
    asm.label("shell_token_boundary");
    a_from(&mut asm, SH_GCOUNT_ADDR);
    asm.i(Instr::Inc8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(&mut asm, SH_GCOUNT_ADDR);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(n_gen_tokens),
    });
    asm.jp(Some(Cond::Z), "shell_gen_done");
    a_from(&mut asm, SH_TFULL_ADDR);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::NZ), "shell_gen_done");
    asm.call("forward_pass");
    asm.jp(None, "sh_gen_loop");

    asm.label("shell_gen_done");
    map_ui(&mut asm);
    call_abs(&mut asm, ui.gen_end);
    asm.jp(None, "shell_idle");

    // the per-token forward pass as a subroutine
    asm.label("forward_pass");
    emit_state_forward_body(&mut asm, &plan);
    asm.i(Instr::Ret { cond: None });

    emit_state_routines_and_tables(&mut asm, model, plan.emb_bank as u8, Some(sampler));

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
