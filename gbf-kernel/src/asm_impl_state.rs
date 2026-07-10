//! One-token and multi-token forward-pass ROM builders for the LinearState
//! stateful bring-up, generalized to a **parameterized topology** (d_model,
//! d_ff, n_blocks, state_slots, vocab) so the same builder serves the arm-B
//! d64/ff128/4blk/slots64 checkpoint and the S8 distilled
//! d192/ff384/6blk/slots192 student the moment its export lands.
//!
//! Builds a banked MBC5 ROM that computes the canonical integer semantics of
//! [`crate::state_model_ref::IntStateLoweredModel::forward`] — including the
//! exact integer recurrence — for one charset id poked into WRAM by the
//! host, and leaves every semantic checkpoint the topology's WRAM budget
//! allows (state-block norm/acc/state/y dumps, block-0 dumps, final norm,
//! i24 logits, argmax, and the persistent state vector itself) in WRAM for
//! byte-exact comparison against the host evaluator.
//!
//! # Parameterization (vs the fixed d64 bring-up)
//!
//! - **WRAM map**: [`StateWramLayout::plan`] computes every buffer address
//!   from the topology and **asserts the 8 KiB budget**, failing loudly
//!   ([`ModelRomError::WramOverflow`]) instead of silently colliding.
//!   When the full debug surface does not fit it degrades in documented
//!   steps: first the per-block residual dumps are dropped (the final
//!   residual is still gated directly from the live `x` buffer), then the
//!   norm `|x|` scratch and the state out-projection accumulators overlay
//!   the matvec accumulator arena (disjoint lifetimes; the out-acc dump
//!   segment is skipped because the FFN accumulators overwrite it).
//! - **Accumulator widths**: matvecs whose structural per-row bound escapes
//!   i16 (the d192 down projection at fan-in 384) get column-segmented
//!   weight code with 3-byte accumulators and a widened epilogue
//!   (`down_ep24w`); everything else keeps the proven i16 V3 chunks. The
//!   decision comes from [`IntStateLoweredModel::lower`].
//! - **Banked tables**: the embedding, tied-head, and state out-projection
//!   tables split across banks with power-of-two row strides; per-row scale
//!   and decay tables move from bank 0 into a dedicated **params bank** so
//!   the driver bank cannot overflow at large topologies.
//! - **Bank count**: up to 512 MBC5 banks; bank switches past 255 write
//!   the ROMB1 high-bit register.
//! - **Norm**: `mean = floor(ss / d_model)` runs as shift-only when
//!   `d_model` is a power of two (the pinned d64 behavior, bit-identical)
//!   and as shift + odd-constant byte-serial division otherwise (d192:
//!   `>> 6` then `/ 3`).
//!
//! The **state vector lives in WRAM at `layout.state`** (`state_slots` x
//! i32 LE, two's complement) and persists across tokens: the one-token ROM
//! never initializes it (the host pokes it), while the multi-token ROM
//! zeroes it once before the generation loop and then lets it evolve
//! on-device for the whole run.
//!
//! Decode: argmax is the default (planv0 v0 decode pin). The sampling
//! variant additionally emits the integer top-k/temperature sampler pinned
//! in [`crate::decode`] and feeds the sampled id back; argmax is still
//! computed and dumped.

use std::collections::BTreeMap;

use gbf_asm::encoder::EncodedSection;
use gbf_asm::isa::{
    AluSrc8, BitIndex, CbTarget, Cond, HighDirectOffset, IncDec8Target, Instr, Reg8, Reg16Addr,
    Reg16Data, Reg16Stack,
};
use gbf_asm::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};
use gbf_asm::rom::{CartridgeHeader, ENTRY_POINT, RomSize, assemble_rom};
use gbf_asm::section::SectionId;

use crate::asm_impl_model::{
    BANK_BYTES, CHUNK_ENTRY, DIV_NUM, DIV_T1, DIV_T2, IPTR, LANE, MBC5_ROMB0, MBC5_ROMB1,
    MODEL_STACK_TOP, ModelAsm, ModelRomError, OPTR, PTR, ROWCNT, SIGN, SPSAVE, SPTR,
    V2_MATRIX_END_WIDE, V2_ROW_END_WIDE, V2_SEG_MID, V2_TABLE_LEN, XPTR, a_from, a_to,
    abs_de_store_sign, build_matvec_chunks_at, build_matvec_chunks_wide, build_matvec_stream_i16,
    build_matvec_stream_wide, emit_copy_bytes, emit_mul16, emit_mul16x8, emit_udiv254, ld_r_imm,
    ld_rr, ld16, load_de_via_ptr, mem_add, mem_copy, mem_shl1, mem_shr1, mem_sub_into,
    smallest_rom_size, zero_mem,
};
use crate::state_model_ref::{AccWidth, IntStateLoweredModel, StateTopology};

/// Bank-0 driver window capacity in bytes: the driver is assembled at
/// [`ENTRY_POINT`] and must end before the switchable-bank chunk entry at
/// [`CHUNK_ENTRY`] (the build fails loudly with
/// [`ModelRomError::DriverOverflowsBank0`] past this bound). Exposed so
/// evidence runners can report the real headroom.
pub const STATE_DRIVER_BANK_CAPACITY: usize = CHUNK_ENTRY as usize - ENTRY_POINT as usize;

// ---------------------------------------------------------------------------
// fixed WRAM anchors (topology-independent)
// ---------------------------------------------------------------------------

/// Activation buffer base (u8 zero point 128). Fixed and page-aligned: the
/// head routine indexes it single-page and the weight chunks embed it as
/// their pop-stream seed. Must hold max(d_model, d_ff) bytes below the
/// shared scratch page at [`SCRATCH_A_BASE`].
pub const S_ACT_BASE: u16 = 0xC000;
/// Shared dense scratch page A (asm_impl_model consts: SPSAVE, MUL_R, ...).
const SCRATCH_A_BASE: u16 = 0xC280;
const SCRATCH_A_END: u16 = 0xC2E0;

// control bytes (fixed block right after scratch A)
/// Input context id; the host pokes this before running.
pub const S_INPUT_ADDR: u16 = 0xC2E0;
/// Argmax id.
pub const S_ARGMAX_ADDR: u16 = 0xC2E1;
/// Done flag (1 when the run is complete).
pub const S_DONE_ADDR: u16 = 0xC2E2;
/// Multi-token loop counter.
pub const S_TOKEN_IDX_ADDR: u16 = 0xC2E3;
/// Sampled id for the current token (sampling variant only).
pub const S_SAMPLED_ADDR: u16 = 0xC2E4;
/// XorShift16 RNG state (2 bytes LE; host pokes the seed; 0 -> 1 on entry).
pub const S_RNG_ADDR: u16 = 0xC2E6;
/// Selected MoE expert index for the current block (u8). Written by the ROM
/// fixed-point router at each MoE block; read by the runtime expert dispatch.
/// Reused per block. Zero/unused on dense (non-MoE) models.
pub const S_EXPERT_SEL_ADDR: u16 = 0xC2E8;
/// High byte of the current input token id (wide-vocab subword models). The
/// low byte lives at [`S_INPUT_ADDR`]; together they form the u16 embedding
/// index. Written by the wide-id feedback loop under [`LogitPaging::Paged`].
/// Always 0 on the charset [`LogitPaging::SinglePage`] path (ids < 256), so the
/// SinglePage embedding lookup / feedback stays byte-identical.
pub const S_INPUT_HI_ADDR: u16 = 0xC2E9;
/// High byte of the PICKED id (sampler `S_SAMPLED_ADDR` low byte, or the
/// paged-argmax `S_ARGMAX_ADDR` low byte — only one epilogue runs per ROM) for
/// the current token (wide-vocab). Together with the low byte it is the u16
/// token id the subword render table indexes and the wide-id feedback loop
/// re-embeds. Zero on the charset SinglePage path.
pub const S_SAMPLED_HI_ADDR: u16 = 0xC2EA;
const CTRL_END: u16 = 0xC2EB;

// scratch page B (state-ROM-owned; fixed block 0xC300..0xC3C0)
const NORM_SS7: u16 = 0xC300; // 7 bytes sum of squares
const ISQ_IN6: u16 = 0xC308; // 6 bytes
const ISQ_REM6: u16 = 0xC310; // 6 bytes
const ISQ_T16: u16 = 0xC318; // 6 bytes
const ISQ_ROOT6: u16 = 0xC320; // 6 bytes
const NORM_R3: u16 = 0xC328; // 3 bytes (rms raw)
const NORM_D5: u16 = 0xC330; // 5 bytes (8r)
const NORM_D25: u16 = 0xC338; // 5 bytes (16r)
const DIV5_NUM: u16 = 0xC340; // 5 bytes
const DIV5_T1: u16 = 0xC348; // 5 bytes
const DIV5_T2: u16 = 0xC350; // 5 bytes
const SQ_T: u16 = 0xC358; // 4 bytes squaring temp
const ST_H: u16 = 0xC360; // 4 bytes state temp
const ST_P: u16 = 0xC368; // 5 bytes decay product
const ST_M: u16 = 0xC370; // 4 bytes delta m
const SIGN2: u16 = 0xC378; // 1 byte (state sign)
const HI8: u16 = 0xC379; // 1 byte (norm squaring high byte)
const DPTR: u16 = 0xC37A; // 2 bytes decay table pointer
const HPTR: u16 = 0xC37C; // 2 bytes state pointer
const WPTR: u16 = 0xC37E; // 2 bytes weight pointer
const ACC4_HI: u8 = 0x80; // smv out-matvec accumulator, HRAM 0xFF80..=0xFF83 (LDH: 3-cycle)
const OEP_A: u16 = 0xC388; // 5 bytes out-epilogue product
const XP2: u16 = 0xC390; // 2 bytes secondary pointer
const YPTR: u16 = 0xC394; // 2 bytes y dump pointer
const SC2: u16 = 0xC396; // 2 bytes out-epilogue scale
const ROWCNT2: u16 = 0xC398; // 2 bytes 16-bit row counter (d_ff rows)
const CHUNK_CNT: u16 = 0xC39A; // 1 byte chunk-run loop counter
const CHUNK_BANK: u16 = 0xC39C; // 2 bytes chunk-run bank number (lo, hi)
// V2 dispatch scratch (only live during a `matvec_v2` walk; the handlers never
// touch anything else here). CHUNK_BANK doubles as the V2 stream bank; SPSAVE
// holds the caller's return-address stack while SP walks the activations.
const WV2_ACC: u16 = 0xC384; // 3 bytes wide (i24) row accumulator (free gap)
const WV2_PK: u16 = 0xC39B; // 1 byte packed-trit decode temp (free gap)
const WV2_OUT: u16 = 0xC39E; // 2 bytes output pointer (free gap after CHUNK_BANK)

// MoE fixed-point router scratch (same fixed page; live only during the router
// routine, which runs at each MoE block BEFORE norm24/up-matvec touch `l.acc`
// and long before the sampling decode uses this page — disjoint lifetimes, so
// these overlay the sampling-decode scratch below). The wide i64 accumulators
// (running acc, product magnitude, argmax best) and the per-rank hidden_q live
// in the matvec accumulator arena `l.acc`; only these small pointers/counters
// live in the fixed page.
const RT_XPTR: u16 = 0xC3A0; // 2 bytes residual (x_i24) pointer
const RT_WPTR: u16 = 0xC3A2; // 2 bytes weight (win_q/wout_q) pointer
const RT_CCNT: u16 = 0xC3A4; // 1 byte inner column counter (d_model or rank)
const RT_SIGN: u16 = 0xC3A5; // 1 byte running term sign (0 = +, 1 = -)
const RT_K: u16 = 0xC3A6; // 1 byte rank / expert loop counter
const RT_BESTE: u16 = 0xC3A7; // 1 byte argmax best expert index
const RT_ROFF: u16 = 0xC3A8; // 2 bytes router-table base (CHUNK_ENTRY + off)
const RT_SIGN2: u16 = 0xC3AA; // 1 byte second operand sign (x / hidden_q)
const RT_E: u16 = 0xC3AB; // 1 byte expert loop counter (phase 2)
const RT_HAVE: u16 = 0xC3AC; // 1 byte "argmax seeded" flag
// Cached dispatch entry for the selected expert (12 bytes: up_bank, up_bc,
// up_scale, down_bank, down_bc, down_scale — all 16-bit LE). Read from the
// bank-0 `moe_disp_b{block}` table after the router picks EXPERT_SEL; survives
// the up/down matvec + epilogue calls (they touch only lower scratch).
const RT_DISP: u16 = 0xC3AD; // MOE_DISP_ENTRY bytes (cached selected-expert entry)
/// Bytes per MoE dispatch-table entry (see the `disp_data` build): up_bank,
/// up_bc, up_scale, down_bank, down_bc, down_scale, scale_bank — each u16 LE.
const MOE_DISP_ENTRY: usize = 14;

// sampling-decode scratch (same fixed page)
const SMP_M: u16 = 0xC3A0; // 3 bytes max logit (hi byte sign-flipped)
const SMP_BEST: u16 = 0xC3A4; // 3 bytes pass best (hi byte sign-flipped)
const SMP_BESTID: u16 = 0xC3A7; // 1 byte
const SMP_D: u16 = 0xC3A8; // 3 bytes d = max - best (u24)
const SMP_P: u16 = 0xC3B0; // 5 bytes d * scale product
const SMP_TOT: u16 = 0xC3B6; // 2 bytes weight total
const SMP_THR: u16 = 0xC3B8; // 2 bytes draw threshold
const SMP_CUM: u16 = 0xC3BA; // 2 bytes cumulative weight
const SMP_PASS: u16 = 0xC3BC; // 1 byte pass counter
const SMP_RT: u16 = 0xC3BE; // 2 bytes rng shift temp
const SCRATCH_B_END: u16 = 0xC3C0;

/// Stack arena: SP starts at [`MODEL_STACK_TOP`] (0xDFF0) and the driver's
/// call depth stays shallow; everything above this line is stack-owned.
const STACK_ARENA_BASE: u16 = 0xDF00;
/// End of the WRAM arena (echo RAM above).
const WRAM_END: u16 = 0xE000;
/// Stack top (grows down; well above every buffer).
pub const S_STACK_TOP: u16 = MODEL_STACK_TOP;

// ---------------------------------------------------------------------------
// WRAM layout (topology-driven)
// ---------------------------------------------------------------------------

/// Shell-owned WRAM block (allocated only for the interactive shell ROM).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellWram {
    /// Prompt buffer page base (`lo(addr) == index`).
    pub prompt: u16,
    pub plen: u16,
    pub submit: u16,
    pub kbcur: u16,
    pub joy_cur: u16,
    pub joy_prev: u16,
    pub widx: u16,
    pub gcount: u16,
    pub tcur: u16,
    pub tfull: u16,
    pub ui_row: u16,
    /// End of the zero-initialized shell block `[prompt, end)`.
    pub end: u16,
}

/// Paged-epilogue WRAM block (allocated ONLY when
/// `topology.logit_paging == Paged`). One extra page holds the running top-1
/// argmax id, the top-k heap, and the finalized candidate id/weight arrays; the
/// single 256-byte logit page at [`StateWramLayout::logits`] is reused per
/// output-page. SinglePage layouts leave this `None`, so their WRAM map and ROM
/// bytes are byte-identical to before paging existed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PagedSampler {
    /// Page base (page-aligned; 0x200 bytes reserved).
    pub page: u16,
    /// Running top-1 argmax id (u16 LE, global id up to `LOGIT_PAGED_VOCAB_MAX`).
    pub argmax16: u16,
    /// Running top-1 best logit (i24 LE, 3 bytes; sign-flipped top byte in the
    /// compare, mirrored to plain i24 for the gate).
    pub best_logit: u16,
    /// Heap logits (`HEAP_K_MAX` * 3 bytes LE i24).
    pub heap_logit: u16,
    /// Heap ids (`HEAP_K_MAX` * 2 bytes LE u16).
    pub heap_id: u16,
    /// Live heap entry count (u8).
    pub heap_count: u16,
    /// Finalized candidate ids in selection order (`HEAP_K_MAX` * 2 bytes u16).
    pub samp_ids: u16,
    /// Finalized candidate weights (`HEAP_K_MAX` bytes u8).
    pub samp_wts: u16,
    /// Per-output-page loop scratch (all inside the paged page):
    /// page index (u8), page length (u8), page base id (u16 LE).
    pub pg_idx: u16,
    pub pg_len: u16,
    pub pg_base: u16,
    /// Scratch for heap offer: candidate id (u16), candidate logit (i24
    /// sign-flipped top byte), the worst-slot index (u8), worst logit (i24).
    pub cand_id: u16,
    pub cand_logit: u16,
    pub worst_idx: u16,
    pub worst_logit: u16,
    /// 2-byte scratch for the heap comparator's "other" id (kept distinct from
    /// `best_logit`, which holds the persistent running top-1 logit).
    pub heap_scratch_id: u16,
}

/// The complete topology-driven WRAM map for one stateful ROM, plus the
/// budget facts the evidence report cites. Every buffer the runner pokes or
/// peeks comes from here — nothing about buffer placement is hard-coded
/// beyond the fixed scratch/control anchors documented above.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateWramLayout {
    pub topology: StateTopology,
    pub down_width: AccWidth,
    /// Matvec input activations (u8 zp128), fixed at [`S_ACT_BASE`].
    pub act: u16,
    /// Matvec raw accumulator arena (i16 LE, or 3-byte LE rows for the wide
    /// down projection).
    pub acc: u16,
    /// |x| buffer for the widened norm (u24 LE x d_model). May equal `acc`
    /// (overlay; disjoint lifetimes).
    pub absx: u16,
    /// State out-projection raw accumulators (i32 LE x d_model). May equal
    /// `acc` (overlay); see `sacc_separate`.
    pub sacc: u16,
    /// True when `sacc` survives to the end of the token (peekable gate
    /// segment); false when it overlays the matvec arena.
    pub sacc_separate: bool,
    /// Residual vector x (i24 LE x d_model).
    pub x: u16,
    /// Persistent recurrent state (i32 LE two's complement x state_slots).
    pub state: u16,
    /// Head per-lane product LUT pages (lo / hi / sign-extension).
    pub lut_lo_page: u16,
    /// i24 LE logits x vocab (page-aligned; single page).
    pub logits: u16,
    /// Multi-token output ring (page-aligned, 256 tokens).
    pub out: u16,
    /// Top-k sampler tables (one page): used flags, candidate ids, weights.
    pub samp_used: u16,
    pub samp_ids: u16,
    pub samp_wts: u16,
    /// State-block norm output dump (u8 zp128 x d_model).
    pub dump_snorm: u16,
    /// State in-projection accumulator dump (i16 LE x state_slots).
    pub dump_inacc: u16,
    /// State-block y activation dump (u8 zp128 x d_model).
    pub dump_yact: u16,
    /// Block-0 norm output dump.
    pub dump_norm0: u16,
    /// Block-0 up accumulator dump (i16 LE x d_ff).
    pub dump_upacc0: u16,
    /// Block-0 GELU activation dump (d_ff bytes).
    pub dump_gelu0: u16,
    /// Block-0 down accumulator dump (2- or 3-byte rows per `down_width`).
    pub dump_downacc0: u16,
    /// Final norm output dump (u8 zp128 x d_model).
    pub dump_qdump: u16,
    /// Per-block residual dumps (stride 3 * d_model), when the budget holds
    /// them; the final residual is always gated from the live `x` buffer.
    pub xdump: Option<u16>,
    /// Shell block (interactive shell variant only).
    pub shell: Option<ShellWram>,
    /// Paged-epilogue block (`Some` only when `logit_paging == Paged`).
    pub paged: Option<PagedSampler>,
    /// Total bytes allocated (excluding gaps), for the budget report.
    pub bytes_allocated: usize,
    /// Named allocations, for the untouched-WRAM gate.
    allocations: Vec<(u16, u16)>,
}

/// Bump allocator over the non-reserved WRAM regions.
struct WramBump {
    /// (cursor, end) free regions in address order.
    regions: Vec<(u16, u16)>,
    allocations: Vec<(u16, u16)>,
    bytes: usize,
}

impl WramBump {
    fn new(regions: Vec<(u16, u16)>) -> Self {
        Self {
            regions,
            allocations: Vec::new(),
            bytes: 0,
        }
    }

    /// Allocate `len` bytes at `align` (power of two), first-fit.
    fn alloc(&mut self, len: usize, align: u16) -> Option<u16> {
        for (cursor, end) in &mut self.regions {
            let aligned = cursor.checked_add(align - 1)? & !(align - 1);
            let need = u16::try_from(len).ok()?;
            if aligned.checked_add(need).is_some_and(|e| e <= *end) {
                *cursor = aligned + need;
                self.allocations.push((aligned, aligned + need));
                self.bytes += len;
                return Some(aligned);
            }
        }
        None
    }
}

/// Debug-dump surface levels, from strongest to weakest. The planner takes
/// the strongest level that fits the 8 KiB arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DumpLevel {
    /// Separate |x|/out-acc buffers plus per-block residual dumps.
    FullSeparate,
    /// Separate buffers, no per-block residual dumps.
    NoXdumpSeparate,
    /// |x| and out-acc overlay the matvec arena, no per-block dumps.
    NoXdumpOverlay,
}

impl StateWramLayout {
    /// Plan the WRAM map for a topology, asserting the 8 KiB budget. Tries
    /// the strongest debug surface first and degrades in documented steps;
    /// errors loudly when even the minimal working set cannot fit.
    pub fn plan(
        topology: StateTopology,
        down_width: AccWidth,
        with_shell: bool,
    ) -> Result<Self, ModelRomError> {
        topology
            .validate()
            .map_err(|e| ModelRomError::WramOverflow {
                needed: 0,
                budget: usize::from(WRAM_END - 0xC000),
                detail: e.to_string(),
            })?;
        let act_len = topology.d_model.max(topology.d_ff);
        if S_ACT_BASE + act_len as u16 > SCRATCH_A_BASE {
            return Err(ModelRomError::WramOverflow {
                needed: act_len,
                budget: usize::from(SCRATCH_A_BASE - S_ACT_BASE),
                detail: format!(
                    "activation buffer ({act_len} B) overruns the fixed scratch page at {SCRATCH_A_BASE:#06x}"
                ),
            });
        }
        let mut last_err = String::new();
        for level in [
            DumpLevel::FullSeparate,
            DumpLevel::NoXdumpSeparate,
            DumpLevel::NoXdumpOverlay,
        ] {
            match Self::plan_at(topology, down_width, with_shell, level) {
                Ok(layout) => return Ok(layout),
                Err(detail) => last_err = detail,
            }
        }
        Err(ModelRomError::WramOverflow {
            needed: 0,
            budget: usize::from(WRAM_END - 0xC000),
            detail: format!("no dump level fits: {last_err}"),
        })
    }

    fn plan_at(
        t: StateTopology,
        down_width: AccWidth,
        with_shell: bool,
        level: DumpLevel,
    ) -> Result<Self, String> {
        let d = t.d_model;
        let act_len = d.max(t.d_ff);
        let act_end = S_ACT_BASE + act_len as u16;
        // Free regions: after ACT up to scratch A, and after scratch B up
        // to the stack arena. (Control bytes and both scratch pages are
        // fixed reservations.)
        let mut bump = WramBump::new(vec![
            (act_end, SCRATCH_A_BASE),
            (SCRATCH_B_END, STACK_ARENA_BASE),
        ]);
        let fail = |what: &str| format!("{what} does not fit ({level:?})");

        // Page-aligned buffers first (minimize alignment waste).
        let lut_lo_page = bump
            .alloc(0x300, 0x100)
            .ok_or_else(|| fail("head LUT pages"))?;
        let logits = bump
            .alloc(0x100, 0x100)
            .ok_or_else(|| fail("logits page"))?;
        let out = bump
            .alloc(0x100, 0x100)
            .ok_or_else(|| fail("output ring"))?;
        let samp_used = bump
            .alloc(0x100, 0x100)
            .ok_or_else(|| fail("sampler page"))?;
        let samp_ids = samp_used + t.vocab as u16;
        let samp_wts = samp_ids + 8;
        // Paged epilogue: one extra page holds the running argmax16, the top-k
        // heap, and the finalized candidate arrays (the single logit page is
        // reused per output-page). Allocated only under Paged, so SinglePage
        // layouts are byte-identical.
        let paged = if t.logit_paging == crate::state_model_ref::LogitPaging::Paged {
            let page = bump.alloc(0x200, 0x100).ok_or_else(|| fail("paged page"))?;
            let hk = crate::state_model_ref::HEAP_K_MAX as u16;
            // Layout within the 0x200 block:
            //   argmax16(2) best_logit(3) heap_count(1) | heap_logit(3*40=120)
            //   heap_id(2*40=80) samp_ids(2*40=80) samp_wts(1*40=40)  => 326 B
            let argmax16 = page;
            let best_logit = argmax16 + 2;
            let heap_count = best_logit + 3;
            let heap_logit = heap_count + 1;
            let heap_id = heap_logit + 3 * hk;
            let samp_ids = heap_id + 2 * hk;
            let samp_wts = samp_ids + 2 * hk;
            let pg_idx = samp_wts + hk;
            let pg_len = pg_idx + 1;
            let pg_base = pg_len + 1;
            let cand_id = pg_base + 2;
            let cand_logit = cand_id + 2;
            let worst_idx = cand_logit + 3;
            let worst_logit = worst_idx + 1;
            let heap_scratch_id = worst_logit + 3;
            debug_assert!(
                heap_scratch_id + 2 <= page + 0x200,
                "paged sampler page overflow"
            );
            Some(PagedSampler {
                page,
                argmax16,
                best_logit,
                heap_logit,
                heap_id,
                heap_count,
                samp_ids,
                samp_wts,
                pg_idx,
                pg_len,
                pg_base,
                cand_id,
                cand_logit,
                worst_idx,
                worst_logit,
                heap_scratch_id,
            })
        } else {
            None
        };
        let shell = if with_shell {
            let prompt = bump.alloc(0x100, 0x100).ok_or_else(|| fail("shell page"))?;
            Some(ShellWram {
                prompt,
                plen: prompt + 0x20,
                submit: prompt + 0x21,
                kbcur: prompt + 0x22,
                joy_cur: prompt + 0x23,
                joy_prev: prompt + 0x24,
                widx: prompt + 0x25,
                gcount: prompt + 0x26,
                tcur: prompt + 0x27,
                tfull: prompt + 0x28,
                ui_row: prompt + 0x29,
                end: prompt + 0x30,
            })
        } else {
            None
        };

        // Matvec accumulator arena. When overlaying, it must also hold the
        // |x| buffer (3d) and the out-projection accumulators (4d).
        let overlay = level == DumpLevel::NoXdumpOverlay;
        let down_acc_bytes = match down_width {
            AccWidth::I16 => 2 * d,
            AccWidth::I24 => 3 * d,
        };
        let mut acc_len = (2 * t.d_ff).max(down_acc_bytes).max(2 * t.state_slots);
        if overlay {
            acc_len = acc_len.max(3 * d).max(4 * d);
        }
        let acc = bump
            .alloc(acc_len, 1)
            .ok_or_else(|| fail("matvec accumulators"))?;
        let x = bump.alloc(3 * d, 1).ok_or_else(|| fail("residual x"))?;
        let state = bump
            .alloc(4 * t.state_slots, 1)
            .ok_or_else(|| fail("recurrent state"))?;
        let (absx, sacc, sacc_separate) = if overlay {
            (acc, acc, false)
        } else {
            let absx = bump.alloc(3 * d, 1).ok_or_else(|| fail("|x| buffer"))?;
            let sacc = bump.alloc(4 * d, 1).ok_or_else(|| fail("state out accs"))?;
            (absx, sacc, true)
        };

        // Debug dumps (gate surface).
        let dump_snorm = bump.alloc(d, 1).ok_or_else(|| fail("snorm dump"))?;
        let dump_inacc = bump
            .alloc(2 * t.state_slots, 1)
            .ok_or_else(|| fail("in-acc dump"))?;
        let dump_yact = bump.alloc(d, 1).ok_or_else(|| fail("yact dump"))?;
        let dump_norm0 = bump.alloc(d, 1).ok_or_else(|| fail("norm0 dump"))?;
        let dump_qdump = bump.alloc(d, 1).ok_or_else(|| fail("final norm dump"))?;
        let dump_gelu0 = bump.alloc(t.d_ff, 1).ok_or_else(|| fail("gelu0 dump"))?;
        let dump_downacc0 = bump
            .alloc(down_acc_bytes, 1)
            .ok_or_else(|| fail("downacc0 dump"))?;
        let dump_upacc0 = bump
            .alloc(2 * t.d_ff, 1)
            .ok_or_else(|| fail("upacc0 dump"))?;
        let xdump = match level {
            DumpLevel::FullSeparate => Some(
                bump.alloc(t.n_blocks * 3 * d, 1)
                    .ok_or_else(|| fail("per-block residual dumps"))?,
            ),
            _ => None,
        };

        let mut allocations = bump.allocations.clone();
        // Fixed reservations count as allocated for the untouched gate.
        allocations.push((S_ACT_BASE, act_end));
        allocations.push((SCRATCH_A_BASE, SCRATCH_A_END));
        allocations.push((S_INPUT_ADDR, CTRL_END));
        allocations.push((NORM_SS7, SCRATCH_B_END));
        allocations.push((STACK_ARENA_BASE, WRAM_END));
        allocations.sort_unstable();

        Ok(Self {
            topology: t,
            down_width,
            act: S_ACT_BASE,
            acc,
            absx,
            sacc,
            sacc_separate,
            x,
            state,
            lut_lo_page,
            logits,
            out,
            samp_used,
            samp_ids,
            samp_wts,
            dump_snorm,
            dump_inacc,
            dump_yact,
            dump_norm0,
            dump_upacc0,
            dump_gelu0,
            dump_downacc0,
            dump_qdump,
            xdump,
            shell,
            paged,
            bytes_allocated: bump.bytes
                + usize::from(act_end - S_ACT_BASE)
                + usize::from(SCRATCH_A_END - SCRATCH_A_BASE)
                + usize::from(CTRL_END - S_INPUT_ADDR)
                + usize::from(SCRATCH_B_END - NORM_SS7),
            allocations,
        })
    }

    /// Bytes of the wide-down accumulator rows (2 or 3 per row).
    #[must_use]
    pub fn down_acc_row_bytes(&self) -> usize {
        match self.down_width {
            AccWidth::I16 => 2,
            AccWidth::I24 => 3,
        }
    }

    /// WRAM regions `[start, end)` no token-loop write may touch: the
    /// complement of every allocation over the 8 KiB arena (the stack arena
    /// and fixed scratch are treated as owned, not untouched).
    #[must_use]
    pub fn untouched_regions(&self) -> Vec<(u16, u16)> {
        let mut gaps = Vec::new();
        let mut cursor = 0xC000u16;
        for &(start, end) in &self.allocations {
            if start > cursor {
                gaps.push((cursor, start));
            }
            cursor = cursor.max(end);
        }
        if cursor < WRAM_END {
            gaps.push((cursor, WRAM_END));
        }
        gaps
    }
}

// ---------------------------------------------------------------------------
// public result types
// ---------------------------------------------------------------------------

/// A fully assembled stateful one-token ROM plus the facts the runner needs.
#[derive(Debug, Clone)]
pub struct StateOneTokenRom {
    pub rom: Vec<u8>,
    pub layout: StateWramLayout,
    pub token_start_pc: u16,
    pub token_end_pc: u16,
    pub rom_size: RomSize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
}

/// A fully assembled stateful multi-token generation ROM.
#[derive(Debug, Clone)]
pub struct StateMultiTokenRom {
    pub rom: Vec<u8>,
    pub layout: StateWramLayout,
    pub token_start_pc: u16,
    pub token_boundary_pc: u16,
    pub token_end_pc: u16,
    pub n_tokens: u16,
    pub rom_size: RomSize,
    pub bank_count: u16,
    pub driver_bytes: usize,
    pub weight_code_bytes: usize,
    pub weight_chunk_count: usize,
    pub table_bytes: usize,
}

// ---------------------------------------------------------------------------
// small emit helpers on top of the shared ModelAsm
// ---------------------------------------------------------------------------

/// Map ROM bank `bank` (writes ROMB0 and, for the 9-bit space, ROMB1).
pub(crate) fn set_bank(asm: &mut ModelAsm, bank: u16) {
    ld_r_imm(asm, Reg8::A, (bank & 0xFF) as u8);
    a_to(asm, MBC5_ROMB0);
    ld_r_imm(asm, Reg8::A, (bank >> 8) as u8);
    a_to(asm, MBC5_ROMB1);
}

/// `ptr` variable += `k` (16-bit).
/// Initialize a pointer variable with an immediate address.
fn ptr_init(asm: &mut ModelAsm, ptr: u16, value: u16) {
    ld_r_imm(asm, Reg8::A, (value & 0xFF) as u8);
    a_to(asm, ptr);
    ld_r_imm(asm, Reg8::A, (value >> 8) as u8);
    a_to(asm, ptr + 1);
}

/// HL := (ptr); copy `n` bytes (hl+) -> fixed `dst`; (ptr) := HL.
fn load_via_ptr_to(asm: &mut ModelAsm, ptr: u16, dst: u16, n: u16) {
    a_from(asm, ptr);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, ptr + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    for k in 0..n {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, dst + k);
    }
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, ptr);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, ptr + 1);
}

/// HL := (ptr); copy `n` bytes fixed `src` -> (hl+); (ptr) := HL.
fn store_via_ptr_from(asm: &mut ModelAsm, ptr: u16, src: u16, n: u16) {
    a_from(asm, ptr);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, ptr + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    for k in 0..n {
        a_from(asm, src + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, ptr);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, ptr + 1);
}

/// Two's-complement negate of `n` bytes at `addr` (little-endian).
fn neg_mem(asm: &mut ModelAsm, addr: u16, n: u16) {
    for k in 0..n {
        a_from(asm, addr + k);
        ld_rr(asm, Reg8::B, Reg8::A);
        ld_r_imm(asm, Reg8::A, 0);
        if k == 0 {
            asm.i(Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            });
        } else {
            asm.i(Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            });
        }
        a_to(asm, addr + k);
    }
}

/// Byte-wise `dst ^= src` over `n` bytes (both fixed addresses).
fn xor_mem(asm: &mut ModelAsm, dst: u16, src: u16, n: u16) {
    for k in 0..n {
        a_from(asm, src + k);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, dst + k);
        asm.i(Instr::XorA {
            src: AluSrc8::Reg(Reg8::B),
        });
        a_to(asm, dst + k);
    }
}

/// Ripple `adc 0` through `n` bytes at `addr` (carry must be live).
fn carry_ripple(asm: &mut ModelAsm, addr: u16, n: u16) {
    for k in 0..n {
        a_from(asm, addr + k);
        asm.i(Instr::AdcA {
            src: AluSrc8::Imm(0),
        });
        a_to(asm, addr + k);
    }
}

/// Emit `call copy_bytes` runs covering `len` bytes (splits past 256; a
/// 256-byte run passes B = 0).
fn emit_copy16(asm: &mut ModelAsm, mut src: u16, mut dst: u16, mut len: usize) {
    while len > 0 {
        let run = len.min(256);
        ld16(asm, Reg16Data::HL, src);
        ld16(asm, Reg16Data::DE, dst);
        ld_r_imm(asm, Reg8::B, (run & 0xFF) as u8);
        asm.call("copy_bytes");
        src += run as u16;
        dst += run as u16;
        len -= run;
    }
}

/// Zero `len` bytes at `base` with a BC-counted loop (16-bit lengths).
pub(crate) fn emit_zero16(asm: &mut ModelAsm, base: u16, len: u16) {
    ld16(asm, Reg16Data::HL, base);
    ld16(asm, Reg16Data::BC, len);
    let head = asm.fresh("z16");
    asm.label(&head);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), &head);
}

// ---------------------------------------------------------------------------
// routines
// ---------------------------------------------------------------------------

/// `isqrt48`: ISQ_IN6 (u48) -> NORM_R3 (u24) floor square root. Unrolled 24
/// iterations of the classic shifting algorithm on 6-byte buffers.
fn emit_isqrt48(asm: &mut ModelAsm) {
    asm.label("isqrt48");
    mem_copy(asm, ISQ_REM6, ISQ_IN6, 6);
    zero_mem(asm, ISQ_ROOT6, 6);
    for iter in 0..24u16 {
        let p = 46 - 2 * iter;
        let kb = p / 8;
        let mask = 1u8 << (p % 8);
        let no = format!("isq48_no_{iter}");
        let done = format!("isq48_dn_{iter}");
        mem_copy(asm, ISQ_T16, ISQ_ROOT6, 6);
        a_from(asm, ISQ_T16 + kb);
        asm.i(Instr::OrA {
            src: AluSrc8::Imm(mask),
        });
        a_to(asm, ISQ_T16 + kb);
        mem_sub_into(asm, ISQ_T16, ISQ_REM6, ISQ_T16, 6);
        asm.jr(Some(Cond::C), &no);
        mem_copy(asm, ISQ_REM6, ISQ_T16, 6);
        mem_shr1(asm, ISQ_ROOT6, 6);
        a_from(asm, ISQ_ROOT6 + kb);
        asm.i(Instr::OrA {
            src: AluSrc8::Imm(mask),
        });
        a_to(asm, ISQ_ROOT6 + kb);
        asm.jr(None, &done);
        asm.label(&no);
        mem_shr1(asm, ISQ_ROOT6, 6);
        asm.label(&done);
    }
    mem_copy(asm, NORM_R3, ISQ_ROOT6, 3);
    asm.i(Instr::Ret { cond: None });
}

/// `udiv_norm5`: DIV5_NUM (u40) / NORM_D25 (u40, nonzero) -> A = min(q, 255).
fn emit_udiv_norm5(asm: &mut ModelAsm) {
    asm.label("udiv_norm5");
    // DIV5_T1 = NORM_D25 << 8 (byte shift; D25 < 2^32 so nothing is lost)
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, DIV5_T1);
    for k in 0..4u16 {
        a_from(asm, NORM_D25 + k);
        a_to(asm, DIV5_T1 + 1 + k);
    }
    // clamp check: NUM >= T1 -> 255
    mem_sub_into(asm, DIV5_T2, DIV5_NUM, DIV5_T1, 5);
    asm.jr(Some(Cond::C), "udn5_go");
    ld_r_imm(asm, Reg8::A, 255);
    asm.i(Instr::Ret { cond: None });
    asm.label("udn5_go");
    ld_r_imm(asm, Reg8::C, 0);
    for iter in 0..8u16 {
        let no = format!("udn5_no_{iter}");
        let rot = format!("udn5_rot_{iter}");
        mem_shr1(asm, DIV5_T1, 5);
        mem_sub_into(asm, DIV5_T2, DIV5_NUM, DIV5_T1, 5);
        asm.jr(Some(Cond::C), &no);
        mem_copy(asm, DIV5_NUM, DIV5_T2, 5);
        asm.i(Instr::Scf);
        asm.jr(None, &rot);
        asm.label(&no);
        asm.i(Instr::OrA {
            src: AluSrc8::Reg(Reg8::A),
        }); // carry := 0
        asm.label(&rot);
        asm.i(Instr::Rl {
            target: CbTarget::Reg(Reg8::C),
        });
    }
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::Ret { cond: None });
}

/// `udiv16_odd`: HL (< 256 * odd) / `odd` -> A = quotient (u8), L = remainder,
/// H = 0. Unrolled 8-step restoring division against compile-time shifted
/// divisor immediates. Emitted only for non-power-of-two `d_model`.
fn emit_udiv16_odd(asm: &mut ModelAsm, odd: u16) {
    debug_assert!(odd > 1 && odd <= 255 && odd % 2 == 1);
    asm.label("udiv16_odd");
    ld_r_imm(asm, Reg8::B, 0);
    for bit in (0..8u16).rev() {
        let t = odd << bit;
        let skip = format!("u16o_skip_{bit}");
        ld_rr(asm, Reg8::A, Reg8::L);
        asm.i(Instr::SubA {
            src: AluSrc8::Imm((t & 0xFF) as u8),
        });
        ld_rr(asm, Reg8::E, Reg8::A);
        ld_rr(asm, Reg8::A, Reg8::H);
        asm.i(Instr::SbcA {
            src: AluSrc8::Imm((t >> 8) as u8),
        });
        asm.jr(Some(Cond::C), &skip);
        ld_rr(asm, Reg8::H, Reg8::A);
        ld_rr(asm, Reg8::L, Reg8::E);
        ld_rr(asm, Reg8::A, Reg8::B);
        asm.i(Instr::OrA {
            src: AluSrc8::Imm(1u8 << bit),
        });
        ld_rr(asm, Reg8::B, Reg8::A);
        asm.label(&skip);
    }
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::Ret { cond: None });
}

/// `norm24`: X (i24 x d_model at `l.x`) -> ACT (u8 zp128 x d_model), the
/// canonical integer norm+activation-quant on the widened residual.
/// `mean = floor(ss / d_model)` runs as `>> k` for the power-of-two factor
/// and byte-serial division for the odd factor (bit-identical to the pinned
/// d64 shift when d_model is a power of two).
fn emit_norm_quant24(asm: &mut ModelAsm, l: &StateWramLayout) {
    let d = l.topology.d_model;
    let k = d.trailing_zeros();
    let odd = (d >> k) as u16;
    asm.label("norm24");
    zero_mem(asm, NORM_SS7, 7);
    // pass 1: abs (u24) + square accumulate into the 7-byte sum
    ld_r_imm(asm, Reg8::A, d as u8);
    a_to(asm, LANE);
    ptr_init(asm, PTR, l.x);
    ptr_init(asm, XP2, l.absx);
    asm.label("n24_p1");
    // load x lanes: E = lo, D = mid, C = hi
    a_from(asm, PTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, PTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, PTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, PTR + 1);
    // abs24: if bit7(C): {E,D,C} = 0 - {E,D,C}
    {
        let pos = asm.fresh("n24_abs");
        asm.i(Instr::Bit {
            bit: BitIndex::new(7).expect("bit 7"),
            target: CbTarget::Reg(Reg8::C),
        });
        asm.jr(Some(Cond::Z), &pos);
        asm.i(Instr::XorA {
            src: AluSrc8::Reg(Reg8::A),
        });
        asm.i(Instr::SubA {
            src: AluSrc8::Reg(Reg8::E),
        });
        ld_rr(asm, Reg8::E, Reg8::A);
        ld_r_imm(asm, Reg8::A, 0);
        asm.i(Instr::SbcA {
            src: AluSrc8::Reg(Reg8::D),
        });
        ld_rr(asm, Reg8::D, Reg8::A);
        ld_r_imm(asm, Reg8::A, 0);
        asm.i(Instr::SbcA {
            src: AluSrc8::Reg(Reg8::C),
        });
        ld_rr(asm, Reg8::C, Reg8::A);
        asm.label(&pos);
    }
    // store |x| (3 bytes) to ABSX via XP2
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, HI8);
    a_from(asm, XP2);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, XP2 + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    a_from(asm, HI8);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, XP2);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, XP2 + 1);
    // square: SS += lo16^2 + (hi8*lo16) << 17 + hi8^2 << 32
    ld_rr(asm, Reg8::B, Reg8::D);
    ld_rr(asm, Reg8::C, Reg8::E);
    asm.call("mul16"); // MUL_R = lo16^2, DE preserved
    mem_add(asm, NORM_SS7, crate::asm_impl_model::MUL_R, 4);
    carry_ripple(asm, NORM_SS7 + 4, 3);
    a_from(asm, HI8);
    asm.call("mul16x8"); // C:HL = hi8 * lo16
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, SQ_T);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, SQ_T + 1);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, SQ_T + 2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, SQ_T + 3);
    mem_shl1(asm, SQ_T, 4); // t << 1 (then applied at byte offset 2 = << 16)
    mem_add(asm, NORM_SS7 + 2, SQ_T, 4);
    carry_ripple(asm, NORM_SS7 + 6, 1);
    a_from(asm, HI8);
    ld_r_imm(asm, Reg8::D, 0);
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.call("mul16x8"); // C:HL = hi8^2 (C = 0)
    a_from(asm, NORM_SS7 + 4);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, NORM_SS7 + 4);
    a_from(asm, NORM_SS7 + 5);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::H),
    });
    a_to(asm, NORM_SS7 + 5);
    carry_ripple(asm, NORM_SS7 + 6, 1);
    a_from(asm, LANE);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.jp(Some(Cond::NZ), "n24_p1");

    // mean = SS / d_model = (SS >> k) / odd; ISQ_IN6 = mean + 1
    for _ in 0..k {
        mem_shr1(asm, NORM_SS7, 7);
    }
    if odd > 1 {
        // byte-serial long division MSB -> LSB, remainder carried in C
        ld_r_imm(asm, Reg8::C, 0);
        for i in (0..7u16).rev() {
            ld_rr(asm, Reg8::H, Reg8::C);
            a_from(asm, NORM_SS7 + i);
            ld_rr(asm, Reg8::L, Reg8::A);
            asm.call("udiv16_odd"); // A = q, L = rem
            a_to(asm, NORM_SS7 + i);
            ld_rr(asm, Reg8::C, Reg8::L);
        }
    }
    a_from(asm, NORM_SS7);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    a_to(asm, ISQ_IN6);
    for j in 1..6u16 {
        a_from(asm, NORM_SS7 + j);
        asm.i(Instr::AdcA {
            src: AluSrc8::Imm(0),
        });
        a_to(asm, ISQ_IN6 + j);
    }
    asm.call("isqrt48");
    // NORM_D5 = r << 3; NORM_D25 = r << 4
    mem_copy(asm, NORM_D5, NORM_R3, 3);
    zero_mem(asm, NORM_D5 + 3, 2);
    for _ in 0..3 {
        mem_shl1(asm, NORM_D5, 5);
    }
    mem_copy(asm, NORM_D25, NORM_D5, 5);
    mem_shl1(asm, NORM_D25, 5);

    // pass 2: per-lane rounded division + sign + zero point
    ld_r_imm(asm, Reg8::A, d as u8);
    a_to(asm, LANE);
    ptr_init(asm, PTR, l.x);
    ptr_init(asm, XP2, l.absx);
    ptr_init(asm, OPTR, l.act);
    asm.label("n24_p2");
    // DIV5_NUM = |x| * 254 + 8r  =  (|x| << 8) - (|x| << 1) + NORM_D5
    load_via_ptr_to(asm, XP2, ST_H, 3); // |x| -> ST_H (3 bytes)
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, DIV5_NUM);
    a_to(asm, DIV5_NUM + 4);
    a_to(asm, DIV5_T2 + 3);
    a_to(asm, DIV5_T2 + 4);
    mem_copy(asm, DIV5_NUM + 1, ST_H, 3);
    mem_copy(asm, DIV5_T2, ST_H, 3);
    mem_shl1(asm, DIV5_T2, 5);
    mem_sub_into(asm, DIV5_NUM, DIV5_NUM, DIV5_T2, 5);
    mem_add(asm, DIV5_NUM, NORM_D5, 5);
    asm.call("udiv_norm5");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(128),
    });
    asm.jr(Some(Cond::C), "n24_qok");
    ld_r_imm(asm, Reg8::A, 127);
    asm.label("n24_qok");
    ld_rr(asm, Reg8::B, Reg8::A);
    // sign from the x lane's high byte (PTR walks 3 bytes per lane)
    a_from(asm, PTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, PTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, PTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, PTR + 1);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::D),
    });
    asm.jr(Some(Cond::Z), "n24_pos");
    ld_r_imm(asm, Reg8::A, 128);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(None, "n24_store");
    asm.label("n24_pos");
    ld_r_imm(asm, Reg8::A, 128);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.label("n24_store");
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, OPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, OPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, OPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, OPTR + 1);
    a_from(asm, LANE);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.jp(Some(Cond::NZ), "n24_p2");
    asm.i(Instr::Ret { cond: None });
}

/// `state_update`: for each slot, `h = clamp_i24(sign(h) * ((|h| * decay +
/// 128) >> 8) + scale * acc)` with the exact integer delta. Reads ACC (i16),
/// the params-bank decay/scale tables (caller maps the params bank), and
/// updates STATE in place.
fn emit_state_update(asm: &mut ModelAsm, l: &StateWramLayout, scales_addr: u16, decay_addr: u16) {
    asm.label("state_update");
    ld_r_imm(asm, Reg8::A, l.topology.state_slots as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, HPTR, l.state);
    ptr_init(asm, IPTR, l.acc);
    ptr_init(asm, SPTR, scales_addr);
    ptr_init(asm, DPTR, decay_addr);
    asm.label("su_loop");
    // --- load h (4 bytes, do not advance HPTR yet) ---
    a_from(asm, HPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, HPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    for k in 0..4u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, ST_H + k);
    }
    // sign2 = bit7 of byte3; |h|
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, SIGN2);
    a_from(asm, ST_H + 3);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "su_habs");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SIGN2);
    neg_mem(asm, ST_H, 4);
    asm.label("su_habs");
    // --- decay product: ST_P = decay * |h| (|h| < 2^24 by invariant) ---
    a_from(asm, ST_H);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, ST_H + 1);
    ld_rr(asm, Reg8::D, Reg8::A);
    // decay byte via DPTR
    a_from(asm, DPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, DPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, HI8); // reuse HI8 slot for the decay byte
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, DPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, DPTR + 1);
    a_from(asm, HI8);
    asm.call("mul16x8"); // C:HL = decay * lo16(|h|)
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, ST_P);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, ST_P + 1);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, ST_P + 2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, ST_P + 3);
    a_to(asm, ST_P + 4);
    a_from(asm, ST_H + 2);
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_r_imm(asm, Reg8::D, 0);
    a_from(asm, HI8);
    asm.call("mul16x8"); // C:HL = decay * hi8(|h|) (C = 0)
    a_from(asm, ST_P + 2);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, ST_P + 2);
    a_from(asm, ST_P + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::H),
    });
    a_to(asm, ST_P + 3);
    carry_ripple(asm, ST_P + 4, 1);
    // round-half-away >> 8: |hd| = ST_P[1..4] + carry(ST_P[0] + 128)
    a_from(asm, ST_P);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(128),
    });
    a_from(asm, ST_P + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, ST_H);
    a_from(asm, ST_P + 2);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, ST_H + 1);
    a_from(asm, ST_P + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, ST_H + 2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, ST_H + 3);
    // re-apply sign
    a_from(asm, SIGN2);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "su_m");
    neg_mem(asm, ST_H, 4);
    asm.label("su_m");
    // --- delta m = scale * acc (exact, sign from acc) ---
    load_de_via_ptr(asm, IPTR); // DE = acc (i16)
    abs_de_store_sign(asm); // SIGN = acc < 0; DE = |acc|
    ld_rr(asm, Reg8::B, Reg8::D);
    ld_rr(asm, Reg8::C, Reg8::E);
    load_de_via_ptr(asm, SPTR); // DE = scale raw
    asm.call("mul16"); // MUL_R = scale * |acc| (u32)
    mem_copy(asm, ST_M, crate::asm_impl_model::MUL_R, 4);
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "su_add");
    neg_mem(asm, ST_M, 4);
    asm.label("su_add");
    // --- h' = decayed + m; saturate to +/-(2^23 - 1) ---
    mem_add(asm, ST_H, ST_M, 4);
    a_from(asm, ST_H + 3);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "su_negrange");
    // positive: in range iff byte3 == 0 and bit7(byte2) == 0
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "su_clamp_pos");
    a_from(asm, ST_H + 2);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "su_store");
    asm.label("su_clamp_pos");
    ld_r_imm(asm, Reg8::A, 0xFF);
    a_to(asm, ST_H);
    a_to(asm, ST_H + 1);
    ld_r_imm(asm, Reg8::A, 0x7F);
    a_to(asm, ST_H + 2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, ST_H + 3);
    asm.jr(None, "su_store");
    asm.label("su_negrange");
    // negative: in range iff byte3 == 0xFF and bit7(byte2) == 1
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0xFF),
    });
    asm.jr(Some(Cond::NZ), "su_clamp_neg");
    a_from(asm, ST_H + 2);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "su_store");
    asm.label("su_clamp_neg");
    ld_r_imm(asm, Reg8::A, 0x01);
    a_to(asm, ST_H);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, ST_H + 1);
    ld_r_imm(asm, Reg8::A, 0x80);
    a_to(asm, ST_H + 2);
    ld_r_imm(asm, Reg8::A, 0xFF);
    a_to(asm, ST_H + 3);
    asm.label("su_store");
    store_via_ptr_from(asm, HPTR, ST_H, 4);
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "su_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `state_out_mv`: with a state-table bank mapped and `ROWCNT`/`WPTR` preset
/// by the caller, accumulate the ternary out-projection over the i32 state
/// into SACC via the persistent OPTR. The weight table rows are
/// `state_slots` i8 entries padded to `pad` extra bytes (power-of-two
/// stride), so the caller can walk multiple banks.
/// `ACC4 += (HL..HL+4)` (or `-=` when `sub`) over the carried i24 state slot,
/// advancing the `HL` state cursor by 4. The accumulator lives in HRAM, so each
/// byte is `ldh a,(acc); add/adc a,(hl); ldh (acc),a; inc hl` — the state byte
/// is folded straight into the ALU op via `(hl)` (no temp register) and the
/// accumulator load/store use the 3-cycle `LDH` form. Byte 0 uses `add`/`sub`,
/// bytes 1..4 chain the carry with `adc`/`sbc`. Integer result is identical to
/// the WRAM/absolute version; only the addressing is cheaper.
fn emit_acc4_state_pm(asm: &mut ModelAsm, sub: bool) {
    for k in 0..4u8 {
        asm.i(Instr::LdAFromHighDirect {
            offset: HighDirectOffset::new(ACC4_HI + k),
        });
        let src = AluSrc8::HlIndirect;
        match (sub, k) {
            (false, 0) => asm.i(Instr::AddA { src }),
            (false, _) => asm.i(Instr::AdcA { src }),
            (true, 0) => asm.i(Instr::SubA { src }),
            (true, _) => asm.i(Instr::SbcA { src }),
        }
        asm.i(Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(ACC4_HI + k),
        });
        asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    }
}

/// `state_out_mv`: with a state-table bank mapped and `ROWCNT`/`WPTR` preset
/// (see the caller), dots each ternary out-projection row with the carried i24
/// state into `ACC4`, storing the i32 result through the persistent `OPTR`.
///
/// Register-resident hot loop: `HL` walks the WRAM state slots (so the state
/// byte feeds the accumulate directly as `add a,(hl)` — no temp register), `DE`
/// walks the row's weight bytes in the mapped ROM window, and the 4-byte
/// accumulator lives in HRAM for 3-cycle `LDH` access. This replaces the
/// original interpreter that reloaded and rewrote *both* pointers from scratch
/// memory every column and copied each state slot into `ST_H` before adding.
/// The integer result is byte-for-byte identical (same ordered ternary add/sub;
/// encoding `1 => +1`, other-nonzero `=> -1`, `0 => skip`), only far cheaper.
fn emit_state_out_matvec(asm: &mut ModelAsm, l: &StateWramLayout, pad: u8) {
    asm.label("state_out_mv");
    // DE := (WPTR) weight cursor, once per bank; caller preset WPTR := CHUNK_ENTRY.
    a_from(asm, WPTR);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, WPTR + 1);
    ld_rr(asm, Reg8::D, Reg8::A);
    asm.label("smv_row");
    // ACC4 := 0 (HRAM).
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    for k in 0..4u8 {
        asm.i(Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(ACC4_HI + k),
        });
    }
    ld16(asm, Reg16Data::HL, l.state); // state cursor
    ld_r_imm(asm, Reg8::C, l.topology.state_slots as u8);
    asm.label("smv_col");
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE }); // w = *DE
    asm.i(Instr::Inc16 { dst: Reg16Data::DE });
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "smv_skip");
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(1),
    });
    asm.jr(Some(Cond::NZ), "smv_sub");
    emit_acc4_state_pm(asm, false); // w == +1
    asm.jr(None, "smv_next");
    asm.label("smv_sub");
    emit_acc4_state_pm(asm, true); // w == -1
    asm.jr(None, "smv_next");
    asm.label("smv_skip");
    for _ in 0..4 {
        asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    }
    asm.label("smv_next");
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.jr(Some(Cond::NZ), "smv_col");
    // Row epilogue: store ACC4 (HRAM) -> (OPTR), advancing OPTR. HL (state
    // cursor) is free to clobber (reset next row); DE (weight cursor) untouched.
    a_from(asm, OPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, OPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    for k in 0..4u8 {
        asm.i(Instr::LdAFromHighDirect {
            offset: HighDirectOffset::new(ACC4_HI + k),
        });
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, OPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, OPTR + 1);
    // Skip inter-row padding in the weight stream (advance DE weight cursor).
    if pad > 0 {
        ld_rr(asm, Reg8::A, Reg8::E);
        asm.i(Instr::AddA {
            src: AluSrc8::Imm(pad),
        });
        ld_rr(asm, Reg8::E, Reg8::A);
        ld_rr(asm, Reg8::A, Reg8::D);
        asm.i(Instr::AdcA {
            src: AluSrc8::Imm(0),
        });
        ld_rr(asm, Reg8::D, Reg8::A);
    }
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "smv_row");
    asm.i(Instr::Ret { cond: None });
}

/// `state_out_ep`: per output row, `p = sign(acc2) * min(127,
/// (scale * |acc2| + 32768) >> 16)`; writes `p + 128` to the YACT dump and
/// adds `y_lut[p]` (i16) into the i24 residual. Caller maps the params bank.
fn emit_state_out_epilogue(asm: &mut ModelAsm, l: &StateWramLayout, scales_addr: u16) {
    asm.label("state_out_ep");
    ld_r_imm(asm, Reg8::A, l.topology.d_model as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, IPTR, l.sacc);
    ptr_init(asm, XPTR, l.x);
    ptr_init(asm, YPTR, l.dump_yact);
    ptr_init(asm, SPTR, scales_addr);
    asm.label("sep_loop");
    load_via_ptr_to(asm, IPTR, ST_H, 4); // acc2 (i32)
    load_de_via_ptr(asm, SPTR); // DE = scale raw
    ld_rr(asm, Reg8::A, Reg8::E);
    a_to(asm, SC2);
    ld_rr(asm, Reg8::A, Reg8::D);
    a_to(asm, SC2 + 1);
    // sign + abs of acc2
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, SIGN);
    a_from(asm, ST_H + 3);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "sep_abs");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SIGN);
    neg_mem(asm, ST_H, 4);
    asm.label("sep_abs");
    // saturation shortcut: |acc2| >= 2^23 -> q = 127 (or 0 when scale == 0)
    a_from(asm, ST_H + 3);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "sep_sat");
    a_from(asm, ST_H + 2);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "sep_mul");
    asm.label("sep_sat");
    a_from(asm, SC2);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SC2 + 1);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "sep_sat127");
    ld_r_imm(asm, Reg8::E, 0);
    asm.jp(None, "sep_apply");
    asm.label("sep_sat127");
    ld_r_imm(asm, Reg8::E, 127);
    asm.jp(None, "sep_apply");
    // full path: OEP_A = scale * |acc2| (40-bit), then round >> 16
    asm.label("sep_mul");
    a_from(asm, ST_H);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, ST_H + 1);
    ld_rr(asm, Reg8::B, Reg8::A); // BC = lo16(|acc2|)
    a_from(asm, SC2);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, SC2 + 1);
    ld_rr(asm, Reg8::D, Reg8::A); // DE = scale
    asm.call("mul16"); // MUL_R = scale * lo16 (u32), DE preserved
    mem_copy(asm, OEP_A, crate::asm_impl_model::MUL_R, 4);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, OEP_A + 4);
    a_from(asm, ST_H + 2); // hi7(|acc2|), bit7 clear on this path
    asm.call("mul16x8"); // C:HL = hi7 * scale
    a_from(asm, OEP_A + 2);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, OEP_A + 2);
    a_from(asm, OEP_A + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::H),
    });
    a_to(asm, OEP_A + 3);
    a_from(asm, OEP_A + 4);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, OEP_A + 4);
    // += 0x8000
    a_from(asm, OEP_A + 1);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, OEP_A + 1);
    carry_ripple(asm, OEP_A + 2, 3);
    // q = min(127, OEP_A >> 16)
    a_from(asm, OEP_A + 3);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, OEP_A + 4);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "sep_q127");
    a_from(asm, OEP_A + 2);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(128),
    });
    asm.jr(Some(Cond::C), "sep_qok");
    asm.label("sep_q127");
    ld_r_imm(asm, Reg8::A, 127);
    asm.label("sep_qok");
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.label("sep_apply");
    // y dump byte = 128 +/- q; y LUT entry at y_lut + 254 +/- 2q
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "sep_neg");
    ld_r_imm(asm, Reg8::A, 128);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.ld16_label(Reg16Data::HL, "y_lut", 254);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    }); // A = 2q (q <= 127, no carry)
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.jr(None, "sep_fetch");
    asm.label("sep_neg");
    ld_r_imm(asm, Reg8::A, 128);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.ld16_label(Reg16Data::HL, "y_lut", 254);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::SbcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.label("sep_fetch");
    // DE = i16 LUT entry
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    // y dump write (C holds the dump byte)
    a_from(asm, YPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, YPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, YPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, YPTR + 1);
    // sign-extend DE to 3 bytes (B) and add into X[row] (i24)
    ld_r_imm(asm, Reg8::B, 0);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::D),
    });
    asm.jr(Some(Cond::Z), "sep_add");
    ld_r_imm(asm, Reg8::B, 0xFF);
    asm.label("sep_add");
    asm.call("resid_add24");
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "sep_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `resid_add24`: X[row] += B:DE (24-bit wrapping) via XPTR, advancing XPTR
/// by one 3-byte lane. Shared by the state and down epilogues.
fn emit_resid_add24(asm: &mut ModelAsm) {
    asm.label("resid_add24");
    a_from(asm, XPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, XPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, XPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, XPTR + 1);
    asm.i(Instr::Ret { cond: None });
}

/// `udiv254w`: DIV_NUM (u32) / 254 -> B:DE (u24 quotient), the wide twin of
/// `udiv254` for the v2 clamp-free i24 down-delta carrier. The caller must
/// guarantee DIV_NUM < 254 * 2^23 so the quotient fits a signed i24 — the
/// lowering's structural per-row delta bound (`DownDeltaEscapesI24`) proves
/// exactly this for every reachable numerator, so no clamp is emitted.
fn emit_udiv254w(asm: &mut ModelAsm) {
    asm.label("udiv254w");
    // DIV_T1 = 254 << 23 = 0x7F000000
    zero_mem(asm, DIV_T1, 4);
    ld_r_imm(asm, Reg8::A, 0x7F);
    a_to(asm, DIV_T1 + 3);
    // Quotient accumulates in C:DE during the loop (the mem_* helpers
    // clobber A/B/H/L but preserve C/D/E), then moves to B:DE for the
    // shared delta-apply tail.
    ld_r_imm(asm, Reg8::C, 0);
    ld16(asm, Reg16Data::DE, 0);
    for iter in 0..24u16 {
        let no = format!("ud254w_no_{iter}");
        let rot = format!("ud254w_rot_{iter}");
        mem_sub_into(asm, DIV_T2, DIV_NUM, DIV_T1, 4);
        asm.jr(Some(Cond::C), &no);
        mem_copy(asm, DIV_NUM, DIV_T2, 4);
        asm.i(Instr::Scf);
        asm.jr(None, &rot);
        asm.label(&no);
        asm.i(Instr::OrA {
            src: AluSrc8::Reg(Reg8::A),
        });
        asm.label(&rot);
        asm.i(Instr::Rl {
            target: CbTarget::Reg(Reg8::E),
        });
        asm.i(Instr::Rl {
            target: CbTarget::Reg(Reg8::D),
        });
        asm.i(Instr::Rl {
            target: CbTarget::Reg(Reg8::C),
        });
        mem_shr1(asm, DIV_T1, 4);
    }
    ld_rr(asm, Reg8::B, Reg8::C);
    asm.i(Instr::Ret { cond: None });
}

/// Shared epilogue tail: |delta| in DE (u16, `wide = false`; B is zeroed
/// here) or B:DE (u24, `wide = true`; B live from `udiv254w`), SIGN live.
/// Sign-extend/negate into B:DE and add into X[row] (i24 wrapping),
/// advancing XPTR. The narrow emission is byte-identical to the pre-v2 form.
fn emit_delta_apply24(asm: &mut ModelAsm, neg_label: &str, add_label: &str, wide: bool) {
    if !wide {
        ld_r_imm(asm, Reg8::B, 0);
    }
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), add_label);
    asm.label(neg_label);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_r_imm(asm, Reg8::A, 0);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_r_imm(asm, Reg8::A, 0);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::B, Reg8::A);
    asm.label(add_label);
    asm.call("resid_add24");
}

// ---------------------------------------------------------------------------
// MoE fixed-point router (deploy step 4)
// ---------------------------------------------------------------------------

/// `dst[0..n] := 0` (byte fill). Clobbers A.
fn emit_mem_zero_n(asm: &mut ModelAsm, dst: u16, n: usize) {
    ld_r_imm(asm, Reg8::A, 0);
    for i in 0..n {
        a_to(asm, dst + i as u16);
    }
}

/// `dst[0..n] := src[0..n]` (byte copy). Clobbers A.
fn emit_mem_copy_n(asm: &mut ModelAsm, dst: u16, src: u16, n: usize) {
    for i in 0..n {
        let i = i as u16;
        a_from(asm, src + i);
        a_to(asm, dst + i);
    }
}

/// Call one of the shared 8-byte mem-op routines (`rt_add8`, `rt_sub8`,
/// `rt_neg8`, `rt_copy8`) with `HL = dst`, `DE = src`. Clobbers A, B, DE, HL.
fn emit_m8(asm: &mut ModelAsm, routine: &str, dst: u16, src: u16) {
    ld16(asm, Reg16Data::HL, dst);
    ld16(asm, Reg16Data::DE, src);
    asm.call(routine);
}

/// Call `rt_zero8` with `HL = dst`. Clobbers A, HL.
fn emit_zero8(asm: &mut ModelAsm, dst: u16) {
    ld16(asm, Reg16Data::HL, dst);
    asm.call("rt_zero8");
}

/// `mag[0..n] := |src[0..n]|` and store the sign (0 = non-negative, 1 =
/// negative) of the signed `n`-byte little-endian `src` into `sign_addr`.
/// Two's-complement negate when the top byte's bit 7 is set. Clobbers A, B.
fn emit_abs_to_mag(asm: &mut ModelAsm, mag: u16, src: u16, n: usize, sign_addr: u16) {
    let neg = asm.fresh("rt_neg");
    let done = asm.fresh("rt_absdone");
    // sign := (src[n-1] & 0x80) ? 1 : 0
    a_from(asm, src + (n - 1) as u16);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), &neg);
    // non-negative: mag := src, sign := 0
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, sign_addr);
    if n == 8 {
        emit_m8(asm, "rt_copy8", mag, src);
    } else {
        emit_mem_copy_n(asm, mag, src, n);
    }
    asm.jr(None, &done);
    // negative: mag := -src, sign := 1
    asm.label(&neg);
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, sign_addr);
    if n == 8 {
        emit_m8(asm, "rt_neg8", mag, src);
    } else {
        emit_negate_n(asm, mag, src, n);
    }
    asm.label(&done);
}

/// `dst[0..n] := -src[0..n]` (two's complement negate into a separate buffer).
/// Clobbers A, B. Correct multi-byte: `dst = 0 - src` with a borrow chain,
/// loading each `src` byte into `B` first so `A` can be zeroed as the minuend
/// (LD does not touch flags, so the borrow survives).
fn emit_negate_n(asm: &mut ModelAsm, dst: u16, src: u16, n: usize) {
    for i in 0..n {
        let i = i as u16;
        a_from(asm, src + i);
        ld_rr(asm, Reg8::B, Reg8::A);
        ld_r_imm(asm, Reg8::A, 0);
        asm.i(if i == 0 {
            Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            }
        } else {
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            }
        });
        a_to(asm, dst + i);
    }
}

/// Load the little-endian 16-bit limb at `addr` into `BC` (`C` = low byte).
fn emit_load_bc(asm: &mut ModelAsm, addr: u16) {
    a_from(asm, addr);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, addr + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
}

/// Load the little-endian 16-bit limb at `addr` into `DE` (`E` = low byte).
fn emit_load_de(asm: &mut ModelAsm, addr: u16) {
    a_from(asm, addr);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, addr + 1);
    ld_rr(asm, Reg8::D, Reg8::A);
}

/// Emit the shared `rt_magmul` routine: `prod[0..8] := mag_a[0..4] *
/// mag_b[0..8]` keeping only the low 64 bits (mod 2^64, matching the host's
/// wrapping `i64` product). Unsigned schoolbook over 16-bit limbs using the
/// shared `mul16` routine (BC*DE -> MUL_R u32). `mag_a` is 2 limbs, `mag_b` is
/// 4 limbs; partials at limb offset `>= 4` are dropped (they only affect the
/// bits at or above 64). Emitted ONCE (fixed `l.acc`-relative operands) and
/// `call`ed from both router phases. Clobbers A, BC, HL; uses `MUL_R`.
fn emit_mag_mul(asm: &mut ModelAsm, prod: u16, mag_a: u16, mag_b: u16) {
    use crate::asm_impl_model::MUL_R;
    asm.label("rt_magmul");
    emit_mem_zero_n(asm, prod, 8);
    for i in 0..2usize {
        for j in 0..4usize {
            if i + j >= 4 {
                continue;
            }
            // MUL_R (u32) = mag_a_limb[i] * mag_b_limb[j]
            emit_load_bc(asm, mag_a + (2 * i) as u16);
            emit_load_de(asm, mag_b + (2 * j) as u16);
            asm.call("mul16");
            // prod[2*(i+j)..] += MUL_R (4 bytes), carry to the end (8 bytes).
            let off = (2 * (i + j)) as u16;
            let n = 8 - off as usize; // add MUL_R[0..min(4,n)], then ripple carry
            let add_bytes = n.min(4);
            for b in 0..add_bytes {
                a_from(asm, MUL_R + b as u16);
                ld_rr(asm, Reg8::B, Reg8::A);
                a_from(asm, prod + off + b as u16);
                asm.i(if b == 0 {
                    Instr::AddA {
                        src: AluSrc8::Reg(Reg8::B),
                    }
                } else {
                    Instr::AdcA {
                        src: AluSrc8::Reg(Reg8::B),
                    }
                });
                a_to(asm, prod + off + b as u16);
            }
            // ripple the final carry through the remaining high bytes
            for b in add_bytes..n {
                a_from(asm, prod + off + b as u16);
                asm.i(Instr::AdcA {
                    src: AluSrc8::Imm(0),
                });
                a_to(asm, prod + off + b as u16);
            }
        }
    }
    asm.i(Instr::Ret { cond: None });
}

/// Copy `n` bytes from `(HL)` to fixed `dst`, advancing `HL` by `n` (HLI).
/// Clobbers A; leaves `HL = HL_in + n` so the caller can store it back as an
/// advanced pointer. Clobbers A.
fn emit_copy_hl_to(asm: &mut ModelAsm, dst: u16, n: usize) {
    for i in 0..n {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, dst + i as u16);
    }
}

/// `HL += imm` (16-bit immediate) via `DE`. Clobbers A, DE.
fn emit_add_hl_imm(asm: &mut ModelAsm, imm: u16) {
    if imm == 0 {
        return;
    }
    ld16(asm, Reg16Data::DE, imm);
    asm.i(Instr::AddHl { src: Reg16Data::DE });
}

/// `HL += stride * (byte at k_addr)` via the shared `rt_addk` routine (`HL +=
/// DE * B`). Clobbers A, B, DE. Keeps the router body small (7 call sites).
fn emit_add_hl_k_times(asm: &mut ModelAsm, stride: u16, k_addr: u16) {
    ld16(asm, Reg16Data::DE, stride);
    a_from(asm, k_addr);
    ld_rr(asm, Reg8::B, Reg8::A);
    asm.call("rt_addk");
}

/// Emit the shared `rt_addk` routine: `HL += DE * B` (B a small count). Runs
/// `B` add-DE iterations. Clobbers A, B.
fn emit_rt_addk(asm: &mut ModelAsm) {
    asm.label("rt_addk");
    let loop_l = "rt_addk_loop";
    let done = "rt_addk_done";
    asm.label(loop_l);
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), done);
    asm.i(Instr::AddHl { src: Reg16Data::DE });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, loop_l);
    asm.label(done);
    asm.i(Instr::Ret { cond: None });
}

/// `dst[0..n] <<= sh` (logical multi-byte left shift by `sh` bits) via a
/// runtime `sh`-iteration loop over a single-bit multi-byte shift (`sla` byte 0,
/// `rl` the higher bytes). Runtime-looped (not unrolled) to keep the bank-0
/// driver small: `sh = 11` unrolled is ~260 bytes. Clobbers A, B.
fn emit_shl_n(asm: &mut ModelAsm, dst: u16, n: usize, sh: u32) {
    let loop_l = asm.fresh("rt_shl");
    let done = asm.fresh("rt_shl_done");
    ld_r_imm(asm, Reg8::B, sh as u8);
    asm.label(&loop_l);
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), &done);
    for i in 0..n {
        a_from(asm, dst + i as u16);
        if i == 0 {
            asm.i(Instr::Sla {
                target: CbTarget::Reg(Reg8::A),
            });
        } else {
            asm.i(Instr::Rl {
                target: CbTarget::Reg(Reg8::A),
            });
        }
        a_to(asm, dst + i as u16);
    }
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(None, &loop_l);
    asm.label(&done);
}

/// `acc[0..8] := round_half_away(acc >> 16)` (Q16.16 -> integer, matching
/// `FixedRouter`'s `hidden_q`). Computes `mag = (|acc| + 2^15) >> 16`, then
/// re-applies the sign. `tmp` is an 8-byte scratch buffer (caller-owned).
/// Clobbers A, B.
fn emit_round_half_away_shift16(asm: &mut ModelAsm, acc: u16, tmp: u16) {
    let pos = asm.fresh("rt_rpos");
    let store = asm.fresh("rt_rstore");
    // was_neg := acc sign; tmp := |acc|
    a_from(asm, acc + 7);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::Z), &pos);
    emit_m8(asm, "rt_neg8", tmp, acc);
    asm.jr(None, &store);
    asm.label(&pos);
    emit_m8(asm, "rt_copy8", tmp, acc);
    asm.label(&store);
    // tmp += 2^15 (add 0x8000 at byte 1, ripple carry)
    a_from(asm, tmp + 1);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, tmp + 1);
    for b in 2..8u16 {
        a_from(asm, tmp + b);
        asm.i(Instr::AdcA {
            src: AluSrc8::Imm(0),
        });
        a_to(asm, tmp + b);
    }
    // tmp >>= 16 (drop low 2 bytes): tmp[i] := tmp[i+2], top 2 bytes := 0
    for i in 0..6u16 {
        a_from(asm, tmp + i + 2);
        a_to(asm, tmp + i);
    }
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, tmp + 6);
    a_to(asm, tmp + 7);
    // acc := was_neg ? -tmp : tmp
    let apply_pos = asm.fresh("rt_apos");
    let apply_done = asm.fresh("rt_adone");
    a_from(asm, acc + 7);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::Z), &apply_pos);
    emit_m8(asm, "rt_neg8", acc, tmp);
    asm.jr(None, &apply_done);
    asm.label(&apply_pos);
    emit_m8(asm, "rt_copy8", acc, tmp);
    asm.label(&apply_done);
}

/// Store the signed 8-byte `src` into `HQ[k]` (`hq + k*8`), where `k` is the
/// byte at `k_addr` (small). Clobbers A, B, DE, HL.
fn emit_store_hq_k(asm: &mut ModelAsm, src: u16, hq: u16, k_addr: u16) {
    ld16(asm, Reg16Data::HL, hq);
    emit_add_hl_k_times(asm, 8, k_addr);
    // (HL) := src[0..8]
    for i in 0..8u16 {
        a_from(asm, src + i);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
}

/// Load `HQ[k]` (`hq + k*8`, 8 bytes signed) into `dst`. Clobbers A, B, DE, HL.
fn emit_load_hq_k(asm: &mut ModelAsm, dst: u16, hq: u16, k_addr: u16) {
    ld16(asm, Reg16Data::HL, hq);
    emit_add_hl_k_times(asm, 8, k_addr);
    emit_copy_hl_to(asm, dst, 8);
}

/// `acc[0..8] += prod` if the two operand signs agree (RT_SIGN == RT_SIGN2),
/// else `acc[0..8] -= prod`. Reproduces the host's `acc += (+/-)|w|*|x|` where
/// the term sign is `sign(w) xor sign(x)`. Clobbers A, B.
fn emit_apply_signed(asm: &mut ModelAsm, acc: u16, prod: u16) {
    let sub = asm.fresh("rt_sub");
    let done = asm.fresh("rt_asdone");
    a_from(asm, RT_SIGN);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, RT_SIGN2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::B),
    });
    // A != 0 -> signs differ -> subtract
    asm.jr(Some(Cond::NZ), &sub);
    emit_m8(asm, "rt_add8", acc, prod);
    asm.jr(None, &done);
    asm.label(&sub);
    emit_m8(asm, "rt_sub8", acc, prod);
    asm.label(&done);
}

/// Jump to `le_label` when signed 8-byte `a <= b` (i.e. NOT `a > b`). Used by
/// the argmax's strict-`>` keep test (keep current best on `<=`, take on `>`).
///
/// Signed compare via top-byte sign-flip + unsigned compare: `a > b` (signed)
/// iff `a' > b'` (unsigned) where `a' = a ^ (0x80 << 56)`. `a' > b'` iff the
/// borrow chain `a' - b'` has no final borrow AND `a' != b'`. So jump to
/// `le_label` on `borrow OR equal`.
///
/// Pass 1 accumulates `differs = OR_i (a[i] ^ b[i])` into `E` (sign-flip is
/// XOR-invariant, so raw bytes suffice for equality). Pass 2 runs the `sbc`
/// borrow chain (top byte flipped) and reads the final carry. Clobbers A, B, C,
/// D, E.
fn emit_signed_le(asm: &mut ModelAsm, a: u16, b: u16, le_label: &str) {
    // Pass 1: E := OR_i (a[i] ^ b[i])  (equality flag; E == 0 iff a == b).
    ld_r_imm(asm, Reg8::E, 0);
    for i in 0..8u16 {
        a_from(asm, b + i);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, a + i);
        asm.i(Instr::XorA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.i(Instr::OrA {
            src: AluSrc8::Reg(Reg8::E),
        });
        ld_rr(asm, Reg8::E, Reg8::A);
    }
    // Precompute the sign-flipped top bytes into scratch so the borrow chain
    // does not run a carry-clobbering `xor` mid-chain (LD keeps flags; XOR does
    // not). RT_SIGN := a[7]^0x80, RT_SIGN2 := b[7]^0x80.
    a_from(asm, a + 7);
    asm.i(Instr::XorA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, RT_SIGN);
    a_from(asm, b + 7);
    asm.i(Instr::XorA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, RT_SIGN2);
    // Pass 2: borrow chain a' - b'. Carry after the last sbc is the final borrow
    // (a' < b' unsigned). Only LD/a_from/a_to between the sbc ops (flag-safe).
    for i in 0..8u16 {
        let b_addr = if i == 7 { RT_SIGN2 } else { b + i };
        let a_addr = if i == 7 { RT_SIGN } else { a + i };
        a_from(asm, b_addr);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, a_addr);
        asm.i(if i == 0 {
            Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            }
        } else {
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            }
        });
        // discard the difference byte (we only need the final carry)
    }
    // borrow (carry set) -> a < b -> a <= b -> jump le
    asm.jr(Some(Cond::C), le_label);
    // no borrow: a >= b. If equal (E == 0) -> a <= b -> jump le.
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), le_label);
    // else a > b: fall through (caller's "take").
}

/// `moe_router`: the on-device twin of
/// [`crate::state_model_ref::FixedRouter::route`]. Reproduces the host argmax
/// with PURELY INTEGER arithmetic from the SAME quantized tables (win_q/bin_q/
/// wout_q/bout_q, byte-for-byte the bytes `FixedRouter::param_bytes` produced
/// into the params bank), so host and ROM route identically by construction.
///
/// Caller contract: the **params bank is mapped**, `RT_ROFF` holds
/// `CHUNK_ENTRY + router_off[block]` (the router table's params-bank address),
/// and the residual `l.x` holds the raw pre-norm residual (i24 Q19.5, 3-byte LE
/// per lane). On return `S_EXPERT_SEL_ADDR` holds the selected expert index.
/// The routine reads memory only through `HL`/direct loads (never `SP`), so the
/// stack is untouched and no SP save/restore is needed.
///
/// Wide accumulators live in the matvec accumulator arena `l.acc` (disjoint
/// lifetime: this runs before norm24/up-matvec touch `l.acc`):
///   HQ    = l.acc              hidden_q[k], rank * 8 bytes (i64 LE)
///   ACC   = HQ + rank*8        running i64 accumulator (8)
///   SUM   = ACC + 8            phase-1 inner sum S = sum_c win*x (8)
///   BEST  = SUM + 8            argmax best raw[e] (8, i64)
///   MA    = BEST + 8           magnitude of win_q/wout_q (4)
///   MB    = MA + 4             magnitude of x_i24 / hidden_q (8)
///   PROD  = MB + 8             magnitude product low 64 bits (8)
fn emit_moe_router(asm: &mut ModelAsm, l: &StateWramLayout, moe: &MoePlan, rank: usize) {
    let d_model = moe.d_model;
    let n_experts = moe.n_experts;
    let hq = l.acc;
    let acc = hq + (rank * 8) as u16;
    let sum = acc + 8;
    let best = sum + 8;
    let ma = best + 8;
    let mb = ma + 4;
    let prod = mb + 8;

    // Offsets (bytes) of the four tables inside the router blob (relative to
    // RT_ROFF). win_q: rank*d_model i32; bin_q: rank i64; wout_q: n_experts*rank
    // i32; bout_q: n_experts i64.
    let win_bytes = (rank * d_model * 4) as u16;
    let bin_bytes = (rank * 8) as u16;
    let wout_bytes = (n_experts * rank * 4) as u16;

    // `moe_setup` is the blob ENTRY: it must be emitted first so it sits at
    // CHUNK_ENTRY (0x4000) in the mapped router bank, which is where the driver
    // `call`s. It runs the router (picks EXPERT_SEL), then caches the selected
    // expert's 12-byte dispatch entry (from the `moe_disp` table appended after
    // the code) into RT_DISP. `moe_tables`/`moe_disp` are data labels resolved
    // at the tail of this same blob.
    asm.label("moe_setup");
    asm.ld16_label(Reg16Data::HL, "moe_tables", 0);
    emit_store_hl_to(asm, RT_ROFF);
    asm.call("moe_router");
    asm.ld16_label(Reg16Data::HL, "moe_disp", 0);
    emit_add_hl_k_times(asm, MOE_DISP_ENTRY as u16, S_EXPERT_SEL_ADDR);
    emit_copy_hl_to(asm, RT_DISP, MOE_DISP_ENTRY);
    asm.i(Instr::Ret { cond: None });

    // Shared 8-byte memory-op subroutines (HL = dst, DE = src). Emitted once;
    // the router calls them instead of inlining every 8-byte op, keeping the
    // bank-0 driver within budget. Each preserves nothing but the target
    // buffers; callers reload pointers. `rt_zero8` ignores DE.
    asm.label("rt_zero8");
    ld_r_imm(asm, Reg8::A, 0);
    for _ in 0..8 {
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    asm.i(Instr::Ret { cond: None });
    // rt_copy8: (HL) := (DE)  [dst := src]
    asm.label("rt_copy8");
    for _ in 0..8 {
        asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
        asm.i(Instr::Inc16 { dst: Reg16Data::DE });
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    asm.i(Instr::Ret { cond: None });
    // rt_add8: (HL) += (DE)  (carry chain; LD keeps flags)
    asm.label("rt_add8");
    for i in 0..8 {
        asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
        asm.i(Instr::Inc16 { dst: Reg16Data::DE });
        ld_rr(asm, Reg8::B, Reg8::A);
        asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
        asm.i(if i == 0 {
            Instr::AddA {
                src: AluSrc8::Reg(Reg8::B),
            }
        } else {
            Instr::AdcA {
                src: AluSrc8::Reg(Reg8::B),
            }
        });
        asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
        asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    }
    asm.i(Instr::Ret { cond: None });
    // rt_sub8: (HL) -= (DE)
    asm.label("rt_sub8");
    for i in 0..8 {
        asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
        asm.i(Instr::Inc16 { dst: Reg16Data::DE });
        ld_rr(asm, Reg8::B, Reg8::A);
        asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
        asm.i(if i == 0 {
            Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            }
        } else {
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            }
        });
        asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
        asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    }
    asm.i(Instr::Ret { cond: None });
    // rt_neg8: (HL) := -(DE)  (0 - src, borrow chain)
    asm.label("rt_neg8");
    for i in 0..8 {
        asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
        asm.i(Instr::Inc16 { dst: Reg16Data::DE });
        ld_rr(asm, Reg8::B, Reg8::A);
        ld_r_imm(asm, Reg8::A, 0);
        asm.i(if i == 0 {
            Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            }
        } else {
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            }
        });
        asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
        asm.i(Instr::Inc16 { dst: Reg16Data::HL });
    }
    asm.i(Instr::Ret { cond: None });

    // Emit the shared magnitude-multiply and HL-stride routines once.
    emit_mag_mul(asm, prod, ma, mb);
    emit_rt_addk(asm);
    // `rt_term`: PROD := |MA|*|MB| (low 64); then ACC += PROD if the two operand
    // signs (RT_SIGN, RT_SIGN2) agree, else ACC -= PROD. The shared accumulate
    // step for both router phases (both use ACC as the running i64 accumulator).
    asm.label("rt_term");
    asm.call("rt_magmul");
    emit_apply_signed(asm, acc, prod);
    asm.i(Instr::Ret { cond: None });
    // `rt_abs_ma`: load the 4-byte weight at `(RT_WPTR)` into MA, advance
    // RT_WPTR by 4, then MA := |MA| with sign -> RT_SIGN. Both router phases
    // stream weights this way.
    asm.label("rt_abs_ma");
    emit_load_hl_from(asm, RT_WPTR);
    emit_copy_hl_to(asm, ma, 4);
    emit_store_hl_to(asm, RT_WPTR);
    emit_abs_to_mag(asm, ma, ma, 4, RT_SIGN);
    asm.i(Instr::Ret { cond: None });

    asm.label("moe_router");

    // ---- Phase 1: hidden_q[k] = round_half_away( (bin_q[k] + S<<11) >> 16 ) ----
    // where S = sum_c win_q[k,c] * x_i24[c] (exact factoring of the host's
    // sum_c win_q * (x<<11): shift distributes over the sum with no i64 overflow
    // — S < 2^51, bin_q + S<<11 < 2^62).
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, RT_K); // k = 0
    asm.label("rt_k_loop");
    // ACC := 0 ; WPTR := RT_ROFF + k*d_model*4 ; XPTR := l.x ; CCNT := d_model.
    // ACC is the running i64 sum S = sum_c win_q[k,c]*x_i24[c].
    emit_zero8(asm, acc);
    emit_load_hl_from(asm, RT_ROFF);
    emit_add_hl_k_times(asm, (d_model * 4) as u16, RT_K);
    emit_store_hl_to(asm, RT_WPTR);
    ld16(asm, Reg16Data::HL, l.x);
    emit_store_hl_to(asm, RT_XPTR);
    ld_r_imm(asm, Reg8::A, d_model as u8);
    a_to(asm, RT_CCNT);
    asm.label("rt_c_loop");
    // MA := |win_q(*WPTR)| (4 bytes), advancing WPTR; sign -> RT_SIGN.
    asm.call("rt_abs_ma");
    // MB := |x_i24(*XPTR)| (3 bytes, zero-extended to 8), sign -> RT_SIGN2
    emit_zero8(asm, mb);
    emit_load_hl_from(asm, RT_XPTR);
    emit_copy_hl_to(asm, mb, 3);
    emit_store_hl_to(asm, RT_XPTR); // advance XPTR by 3
    emit_abs_to_mag(asm, mb, mb, 3, RT_SIGN2);
    // ACC += (+/-)|MA|*|MB|
    asm.call("rt_term");
    // CCNT-- ; loop
    a_from(asm, RT_CCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, RT_CCNT);
    asm.jp(Some(Cond::NZ), "rt_c_loop");
    // ACC <<= 11 (S << 11), then ACC += bin_q[k] (hidden_acc at Q32.32).
    emit_shl_n(asm, acc, 8, 11);
    // bin_q[k] at RT_ROFF + win_bytes + k*8 -> SUM (temp), then ACC += SUM
    emit_load_hl_from(asm, RT_ROFF);
    emit_add_hl_imm(asm, win_bytes);
    emit_add_hl_k_times(asm, 8, RT_K);
    emit_copy_hl_to(asm, sum, 8);
    emit_m8(asm, "rt_add8", acc, sum);
    // hidden_q[k] := round_half_away(ACC >> 16) into HQ + k*8
    emit_round_half_away_shift16(asm, acc, best); // uses `best` as |acc| scratch
    // store ACC (now the rounded signed hidden_q) to HQ + k*8
    emit_store_hq_k(asm, acc, hq, RT_K);
    // k++ ; if k < rank loop
    a_from(asm, RT_K);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, RT_K);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(rank as u8),
    });
    asm.jp(Some(Cond::C), "rt_k_loop");

    // ---- Phase 2: raw[e] = bout_q[e] + sum_k wout_q[e,k]*hidden_q[k]; argmax ----
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, RT_E);
    a_to(asm, RT_HAVE); // no best yet
    asm.label("rt_e_loop");
    // ACC := bout_q[e] (at RT_ROFF + win_bytes + bin_bytes + wout_bytes + e*8)
    emit_load_hl_from(asm, RT_ROFF);
    emit_add_hl_imm(asm, win_bytes + bin_bytes + wout_bytes);
    emit_add_hl_k_times(asm, 8, RT_E);
    emit_copy_hl_to(asm, acc, 8);
    // WPTR := RT_ROFF + win_bytes + bin_bytes + e*rank*4 (wout_q row e)
    emit_load_hl_from(asm, RT_ROFF);
    emit_add_hl_imm(asm, win_bytes + bin_bytes);
    emit_add_hl_k_times(asm, (rank * 4) as u16, RT_E);
    emit_store_hl_to(asm, RT_WPTR);
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, RT_K);
    asm.label("rt_e_k_loop");
    // MA := |wout_q(*WPTR)| (4), advancing WPTR; sign -> RT_SIGN.
    asm.call("rt_abs_ma");
    // MB := |hidden_q[k]| (8), sign -> RT_SIGN2
    emit_load_hq_k(asm, mb, hq, RT_K);
    emit_abs_to_mag(asm, mb, mb, 8, RT_SIGN2);
    // ACC += (+/-)|MA|*|MB|
    asm.call("rt_term");
    // k++ ; if k < rank loop
    a_from(asm, RT_K);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, RT_K);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(rank as u8),
    });
    asm.jp(Some(Cond::C), "rt_e_k_loop");
    // argmax: if !HAVE || ACC > BEST (signed strict) -> take (BEST := ACC,
    // BESTE := e, HAVE := 1). Lowest index wins ties (strict `>`).
    let take = asm.fresh("rt_take");
    let keep = asm.fresh("rt_keep");
    a_from(asm, RT_HAVE);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), &take); // HAVE == 0 -> take unconditionally
    // HAVE != 0: if ACC <= BEST -> keep, else fall through to take.
    emit_signed_le(asm, acc, best, &keep);
    asm.label(&take);
    emit_m8(asm, "rt_copy8", best, acc);
    a_from(asm, RT_E);
    a_to(asm, RT_BESTE);
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, RT_HAVE);
    asm.label(&keep);
    // e++ ; if e < n_experts loop
    a_from(asm, RT_E);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, RT_E);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(n_experts as u8),
    });
    asm.jp(Some(Cond::C), "rt_e_loop");
    // EXPERT_SEL := BESTE
    a_from(asm, RT_BESTE);
    a_to(asm, S_EXPERT_SEL_ADDR);
    asm.i(Instr::Ret { cond: None });

    // Blob-local copies of the unsigned multiply (the router runs from a
    // switchable bank; duplicating `mul16`/`mul16x8` here makes the blob fully
    // self-contained — no bank-0 dependency during the router). ~150 bytes of
    // ROM per router bank; ROM is cheap, bank-0 space is not.
    crate::asm_impl_model::emit_mul16x8(asm);
    crate::asm_impl_model::emit_mul16(asm);
}

/// Assemble one block's self-contained router bank: the router code (entry
/// `moe_setup` at CHUNK_ENTRY) followed by the `moe_disp` dispatch table and
/// `moe_tables` fixed-point router tables. The code references those two data
/// blocks by label, so this must be one assembly unit. Runs from the switchable
/// 0x4000 window (mapped by the driver before the `call CHUNK_ENTRY`).
fn build_moe_router_bank(
    l: &StateWramLayout,
    moe: &MoePlan,
    disp_data: &[u8],
    router_tables: &[u8],
) -> Result<Vec<u8>, ModelRomError> {
    let mut asm = ModelAsm::new(CHUNK_ENTRY);
    emit_moe_router(&mut asm, l, moe, moe.rank);
    asm.label("moe_disp");
    asm.bytes(disp_data.to_vec());
    asm.label("moe_tables");
    asm.bytes(router_tables.to_vec());
    let (bytes, _labels) = asm.finish()?;
    if bytes.len() > BANK_BYTES {
        return Err(ModelRomError::ParamsBankOverflow { bytes: bytes.len() });
    }
    Ok(bytes)
}

/// Bank-0 MoE dispatch routines. `moe_up`/`moe_down` run the selected expert's
/// up/down matvec AND set up the following epilogue, all from the cached
/// dispatch entry (14 bytes: up_bank, up_bc, up_scale, down_bank, down_bc,
/// down_scale, scale_bank). First they program the matvec bank + `CHUNK_BANK` +
/// `BC` (via `moe_mv_prog`) and `call` the V2 handler (narrow for up, wide for
/// down); then they map the block's scale bank and load `DE` = the epilogue
/// scale pointer. So the per-block driver is just `call moe_up; call up_ep16`
/// (no inline bank-switch / scale-load code — that is what keeps the paged-vocab
/// driver under the bank-0 budget for BOTH the one-token and multi-token ROMs).
fn emit_moe_bank0_routines(asm: &mut ModelAsm) {
    asm.label("moe_up");
    ld16(asm, Reg16Data::HL, RT_DISP);
    asm.call("moe_mv_prog"); // program + BC from up half (RT_DISP+0..4)
    asm.call("matvec_v2");
    // map scale bank (RT_DISP+12), DE := up_scale (RT_DISP+4)
    a_from(asm, RT_DISP + 12);
    a_to(asm, MBC5_ROMB0);
    a_from(asm, RT_DISP + 13);
    a_to(asm, MBC5_ROMB1);
    emit_load_de(asm, RT_DISP + 4);
    asm.i(Instr::Ret { cond: None });

    asm.label("moe_down");
    ld16(asm, Reg16Data::HL, RT_DISP + 6);
    asm.call("moe_mv_prog"); // program + BC from down half (RT_DISP+6..10)
    asm.call("matvec_v2w");
    a_from(asm, RT_DISP + 12);
    a_to(asm, MBC5_ROMB0);
    a_from(asm, RT_DISP + 13);
    a_to(asm, MBC5_ROMB1);
    emit_load_de(asm, RT_DISP + 10);
    asm.i(Instr::Ret { cond: None });

    // moe_mv_prog: program MBC5 ROMB0/ROMB1 + CHUNK_BANK (bank at (HL),(HL+1))
    // and BC (stream pointer at (HL+2),(HL+3)) from the 4-byte half at (HL).
    asm.label("moe_mv_prog");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, MBC5_ROMB0);
    a_to(asm, CHUNK_BANK);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, MBC5_ROMB1);
    a_to(asm, CHUNK_BANK + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::C, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::B, Reg8::A);
    asm.i(Instr::Ret { cond: None });
}

/// `up_ep16`: DE = scale-table pointer; ROWCNT2 preset to the row count
/// (16-bit, so d_ff can exceed 255). For each row:
/// ACT[row] = GELU_LUT[127 + clamp(round_half_away(scale*acc / 256), -127, 127)].
/// Caller maps the params bank first (GELU LUT itself lives in bank 0).
fn emit_up_epilogue16(asm: &mut ModelAsm, l: &StateWramLayout) {
    use crate::asm_impl_model::MUL_R;
    asm.label("up_ep16");
    ld_rr(asm, Reg8::A, Reg8::E);
    a_to(asm, SPTR);
    ld_rr(asm, Reg8::A, Reg8::D);
    a_to(asm, SPTR + 1);
    ptr_init(asm, IPTR, l.acc);
    ptr_init(asm, OPTR, l.act);
    asm.label("upe16_loop");
    load_de_via_ptr(asm, IPTR);
    abs_de_store_sign(asm);
    ld_rr(asm, Reg8::B, Reg8::D);
    ld_rr(asm, Reg8::C, Reg8::E);
    load_de_via_ptr(asm, SPTR);
    asm.call("mul16");
    // p = min(127, (MUL_R + 128) >> 8)
    a_from(asm, MUL_R);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(128),
    });
    a_from(asm, MUL_R + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, MUL_R + 2);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    a_from(asm, MUL_R + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::D),
    });
    asm.jr(Some(Cond::NZ), "upe16_clamp");
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(128),
    });
    asm.jr(Some(Cond::C), "upe16_pok");
    asm.label("upe16_clamp");
    ld_r_imm(asm, Reg8::E, 127);
    asm.label("upe16_pok");
    // HL = gelu_center +/- E
    asm.ld16_label(Reg16Data::HL, "gelu_lut", 127);
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "upe16_plus");
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::SbcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.jr(None, "upe16_fetch");
    asm.label("upe16_plus");
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.label("upe16_fetch");
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::C });
    a_from(asm, OPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, OPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, OPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, OPTR + 1);
    // 16-bit row counter decrement
    a_from(asm, ROWCNT2);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    a_to(asm, ROWCNT2);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, ROWCNT2 + 1);
    asm.i(Instr::SbcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, ROWCNT2 + 1);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "upe16_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `down_ep24`: DE = scale-table pointer; d_model rows of **i16** accs.
/// X[row] (i24) += (mod 2^24) sign(m) * min(65535, (|m|*2 + 127) / 254)
/// with m = scale * acc — the dense Q-grid formula on the widened residual.
fn emit_down_epilogue24(asm: &mut ModelAsm, l: &StateWramLayout) {
    use crate::asm_impl_model::MUL_R;
    asm.label("down_ep24");
    ld_rr(asm, Reg8::A, Reg8::E);
    a_to(asm, SPTR);
    ld_rr(asm, Reg8::A, Reg8::D);
    a_to(asm, SPTR + 1);
    ld_r_imm(asm, Reg8::A, l.topology.d_model as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, IPTR, l.acc);
    ptr_init(asm, XPTR, l.x);
    asm.label("d24_loop");
    load_de_via_ptr(asm, IPTR);
    abs_de_store_sign(asm);
    ld_rr(asm, Reg8::B, Reg8::D);
    ld_rr(asm, Reg8::C, Reg8::E);
    load_de_via_ptr(asm, SPTR);
    asm.call("mul16");
    // DIV_NUM = (MUL_R << 1) + 127
    mem_copy(asm, DIV_NUM, MUL_R, 4);
    mem_shl1(asm, DIV_NUM, 4);
    a_from(asm, DIV_NUM);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(127),
    });
    a_to(asm, DIV_NUM);
    carry_ripple(asm, DIV_NUM + 1, 3);
    asm.call("udiv254"); // DE = min(q, 65535)
    emit_delta_apply24(asm, "d24_neg", "d24_add", false);
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "d24_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `down_ep24w`: the wide twin of `down_ep24` over **3-byte i24** accs.
/// m = scale * |acc| via mul16 + mul16x8 (the lowering's structural delta
/// bound guarantees the numerator 2m + 127 < 254 * 2^23 < 2^31), then the
/// exact Q19.5 delta X[row] (i24) += (mod 2^24) sign(m) * ((|m|*2 + 127) /
/// 254) carried in a **clamp-free i24** (state-int-semantics.v2; the v1 u16
/// carrier saturated on the real d192 student, bd-2vkqt).
fn emit_down_epilogue24_wide(asm: &mut ModelAsm, l: &StateWramLayout) {
    use crate::asm_impl_model::MUL_R;
    asm.label("down_ep24w");
    ld_rr(asm, Reg8::A, Reg8::E);
    a_to(asm, SPTR);
    ld_rr(asm, Reg8::A, Reg8::D);
    a_to(asm, SPTR + 1);
    ld_r_imm(asm, Reg8::A, l.topology.d_model as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, IPTR, l.acc);
    ptr_init(asm, XPTR, l.x);
    asm.label("d24w_loop");
    load_via_ptr_to(asm, IPTR, ST_H, 3); // acc (i24 LE)
    // sign + abs
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, SIGN);
    a_from(asm, ST_H + 2);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "d24w_abs");
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SIGN);
    neg_mem(asm, ST_H, 3);
    asm.label("d24w_abs");
    // m = scale * |acc|: DIV_NUM = MUL_R(lo16 * scale) + (hi8 * scale) << 16
    a_from(asm, ST_H);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, ST_H + 1);
    ld_rr(asm, Reg8::B, Reg8::A); // BC = lo16(|acc|)
    load_de_via_ptr(asm, SPTR); // DE = scale raw
    asm.call("mul16"); // MUL_R = scale * lo16, DE preserved
    mem_copy(asm, DIV_NUM, MUL_R, 4);
    a_from(asm, ST_H + 2); // hi8(|acc|)
    asm.call("mul16x8"); // C:HL = hi8 * scale (C = 0 by the lowering bound)
    a_from(asm, DIV_NUM + 2);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, DIV_NUM + 2);
    a_from(asm, DIV_NUM + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::H),
    });
    a_to(asm, DIV_NUM + 3);
    // DIV_NUM = (m << 1) + 127 (< 254 * 2^23 by the lowering delta bound)
    mem_shl1(asm, DIV_NUM, 4);
    a_from(asm, DIV_NUM);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(127),
    });
    a_to(asm, DIV_NUM);
    carry_ripple(asm, DIV_NUM + 1, 3);
    asm.call("udiv254w"); // B:DE = q (u24, exact; no clamp)
    emit_delta_apply24(asm, "d24w_neg", "d24w_add", true);
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "d24w_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `emb_copy24`: A = input id LOW byte. Maps the embedding bank holding the
/// id's row (power-of-two `stride` bytes per row, `rows_per_bank` rows per
/// bank) and copies the `3 * d_model`-byte i24 row into X.
///
/// When `wide` (a paged wide-vocab subword model), the id is the u16
/// `S_INPUT_HI_ADDR:S_INPUT_ADDR`; the caller still passes the low byte in `A`,
/// and the routine reads the high byte from `S_INPUT_HI_ADDR`. The bank index
/// `id >> log_rpb` is computed as a full 16-bit shift so ids >= 256 map to the
/// correct embedding bank. The SinglePage (`!wide`) path is byte-identical to
/// the pre-wide emission (id < 256, high byte always 0).
fn emit_emb_copy24(
    asm: &mut ModelAsm,
    l: &StateWramLayout,
    emb_bank0: u16,
    stride: usize,
    wide: bool,
) {
    let rows_per_bank = BANK_BYTES / stride;
    debug_assert!(rows_per_bank.is_power_of_two());
    let log_rpb = rows_per_bank.trailing_zeros();
    let page_shift = stride.trailing_zeros() - 8;
    let mask = (rows_per_bank - 1) as u8;
    asm.label("emb_copy24");
    ld_rr(asm, Reg8::B, Reg8::A);
    if wide {
        // 16-bit id = D:A (D = S_INPUT_HI, A = low byte). bank index =
        // (D:A) >> log_rpb, propagating each bit across the byte boundary via
        // the carry (SRL D; RR A). The result fits the low byte (vocab id space
        // is <= 2^16, and rows_per_bank >= 1 keeps the quotient in 8 bits for
        // the deployed vocab), which lands in A for the emb_bank0 add below.
        a_from(asm, S_INPUT_HI_ADDR);
        ld_rr(asm, Reg8::D, Reg8::A);
        ld_rr(asm, Reg8::A, Reg8::B); // A = low byte
        for _ in 0..log_rpb {
            asm.i(Instr::Srl {
                target: CbTarget::Reg(Reg8::D),
            });
            asm.i(Instr::Rr {
                target: CbTarget::Reg(Reg8::A),
            });
        }
    } else {
        // bank = emb_bank0 + (id >> log_rpb), id < 256 (charset SinglePage).
        for _ in 0..log_rpb {
            asm.i(Instr::Srl {
                target: CbTarget::Reg(Reg8::A),
            });
        }
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((emb_bank0 & 0xFF) as u8),
    });
    a_to(asm, MBC5_ROMB0); // LD (nn),A preserves flags; carry survives
    ld_r_imm(asm, Reg8::A, (emb_bank0 >> 8) as u8); // LD r,n preserves flags
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, MBC5_ROMB1);
    // HL = 0x4000 + (id & mask) * stride
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(mask),
    });
    for _ in 0..page_shift {
        asm.i(Instr::AddA {
            src: AluSrc8::Reg(Reg8::A),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_r_imm(asm, Reg8::L, 0);
    // copy 3 * d_model bytes HL -> X (16-bit count)
    ld16(asm, Reg16Data::DE, l.x);
    ld16(asm, Reg16Data::BC, (3 * l.topology.d_model) as u16);
    asm.label("ec24_copy");
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
    asm.jr(Some(Cond::NZ), "ec24_copy");
    asm.i(Instr::Ret { cond: None });
}

/// One head lane group: with head bank `g` mapped, accumulate lanes
/// `lane_lo..lane_hi` into the `vocab` i24 logits via per-lane product LUT
/// pages (dense construction). Emitted once per head bank because the
/// bank-relative page base is a compile-time immediate.
fn emit_head_group(
    asm: &mut ModelAsm,
    l: &StateWramLayout,
    g: usize,
    lane_lo: usize,
    lane_hi: usize,
) {
    let t = &l.topology;
    let lut_hi = (l.lut_lo_page >> 8) as u8;
    let act_hi = (l.act >> 8) as u8;
    debug_assert_eq!(l.act & 0xFF, 0, "activation page-aligned");
    let lbl = |s: &str| format!("{s}_{g}");
    asm.label(&lbl("head_grp"));
    ld_r_imm(asm, Reg8::A, lane_lo as u8);
    a_to(asm, LANE);
    asm.label(&lbl("hg_lane"));
    // q = ACT[lane] - 128
    a_from(asm, LANE);
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, act_hi);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(128),
    });
    a_to(asm, SIGN); // q byte
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, 0);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), &lbl("hg_qpos"));
    ld_r_imm(asm, Reg8::B, 0xFF);
    asm.label(&lbl("hg_qpos"));
    // ascending half: entries 0..=127
    ld16(asm, Reg16Data::DE, 0);
    ld16(asm, Reg16Data::HL, l.lut_lo_page);
    asm.label(&lbl("hg_asc"));
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_asc"));
    // descending half: entries 255 down to 128
    ld16(asm, Reg16Data::DE, 0);
    ld_r_imm(asm, Reg8::L, 0xFF);
    asm.label(&lbl("hg_desc"));
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::Z), &lbl("hg_desc_done"));
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::L),
    });
    asm.jr(None, &lbl("hg_desc"));
    asm.label(&lbl("hg_desc_done"));
    // sign-extension page
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.jr(Some(Cond::Z), &lbl("hg_sx"));
    asm.i(Instr::Cpl);
    asm.label(&lbl("hg_sx"));
    ld_rr(asm, Reg8::C, Reg8::A);
    ld16(asm, Reg16Data::HL, l.lut_lo_page + 0x200);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.label(&lbl("hg_fillp"));
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_fillp"));
    asm.label(&lbl("hg_filln"));
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_filln"));
    // accumulate: D = head page (0x40 + lane - lane_lo), E over 0..vocab
    a_from(asm, LANE);
    if lane_lo > 0 {
        asm.i(Instr::SubA {
            src: AluSrc8::Imm(lane_lo as u8),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_r_imm(asm, Reg8::E, 0);
    ld16(asm, Reg16Data::HL, l.logits);
    asm.label(&lbl("hg_acc"));
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, lut_hi);
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AddA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AdcA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AdcA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(t.vocab as u8),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_acc"));
    a_from(asm, LANE);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(lane_hi as u8),
    });
    asm.jp(Some(Cond::NZ), &lbl("hg_lane"));
    asm.i(Instr::Ret { cond: None });
}

/// `argmax_v`: scan the `vocab` i24 logits, strict-greater update (lowest
/// index wins ties), signed compare via a sign-flipped top byte.
fn emit_argmax(asm: &mut ModelAsm, l: &StateWramLayout) {
    use crate::asm_impl_model::{ARG_BEST, ARG_CAND};
    asm.label("argmax_v");
    ld16(asm, Reg16Data::HL, l.logits);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_BEST);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_BEST + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::XorA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, ARG_BEST + 2);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, S_ARGMAX_ADDR);
    ld_r_imm(asm, Reg8::C, 1);
    asm.label("amx_loop");
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::XorA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, ARG_CAND + 2);
    for k in [2u16, 1, 0] {
        a_from(asm, ARG_CAND + k);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, ARG_BEST + k);
        asm.i(Instr::CpA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::C), "amx_upd");
        if k > 0 {
            asm.jr(Some(Cond::NZ), "amx_next");
        } else {
            asm.jr(None, "amx_next");
        }
    }
    asm.label("amx_upd");
    mem_copy(asm, ARG_BEST, ARG_CAND, 3);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, S_ARGMAX_ADDR);
    asm.label("amx_next");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(l.topology.vocab as u8),
    });
    asm.jp(Some(Cond::NZ), "amx_loop");
    asm.i(Instr::Ret { cond: None });
}

/// Heap capacity for the paged running top-k: `min(HEAP_K_MAX, vocab)`. Fits
/// u8 (`HEAP_K_MAX = 40`).
fn paged_heap_k(t: &StateTopology) -> u8 {
    crate::state_model_ref::HEAP_K_MAX.min(t.vocab).max(1) as u8
}

/// Paged twin of [`emit_head_group`]: identical per-lane product-LUT build and
/// i24 accumulate, but the accumulate loop runs over the `<= LOGIT_PAGE_IDS`
/// page-local ids of the CURRENT output-page (count read from `pg.pg_len`)
/// rather than the full `vocab`. The head weight bank mapped by the caller is
/// this `(page, group)` combo, so `[DE]` (head page, local id) still lands on
/// `head_i8_row_at(page_base + local_id)[lane]`, and the WRAM logit page is
/// indexed by page-local id (reset per output-page). Emitted only under
/// [`LogitPaging::Paged`]; the SinglePage `head_grp_g` bytes are unchanged.
fn emit_head_group_paged(
    asm: &mut ModelAsm,
    l: &StateWramLayout,
    g: usize,
    lane_lo: usize,
    lane_hi: usize,
) {
    let pg = l.paged.expect("paged layout");
    let lut_hi = (l.lut_lo_page >> 8) as u8;
    let act_hi = (l.act >> 8) as u8;
    debug_assert_eq!(l.act & 0xFF, 0, "activation page-aligned");
    let lbl = |s: &str| format!("{s}_pg_{g}");
    asm.label(&lbl("head_grp"));
    ld_r_imm(asm, Reg8::A, lane_lo as u8);
    a_to(asm, LANE);
    asm.label(&lbl("hg_lane"));
    // q = ACT[lane] - 128
    a_from(asm, LANE);
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, act_hi);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(128),
    });
    a_to(asm, SIGN); // q byte
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, 0);
    asm.i(Instr::Bit {
        bit: BitIndex::new(7).expect("bit 7"),
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), &lbl("hg_qpos"));
    ld_r_imm(asm, Reg8::B, 0xFF);
    asm.label(&lbl("hg_qpos"));
    // ascending half: entries 0..=127
    ld16(asm, Reg16Data::DE, 0);
    ld16(asm, Reg16Data::HL, l.lut_lo_page);
    asm.label(&lbl("hg_asc"));
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_asc"));
    // descending half: entries 255 down to 128
    ld16(asm, Reg16Data::DE, 0);
    ld_r_imm(asm, Reg8::L, 0xFF);
    asm.label(&lbl("hg_desc"));
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::H),
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::Z), &lbl("hg_desc_done"));
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::L),
    });
    asm.jr(None, &lbl("hg_desc"));
    asm.label(&lbl("hg_desc_done"));
    // sign-extension page
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.jr(Some(Cond::Z), &lbl("hg_sx"));
    asm.i(Instr::Cpl);
    asm.label(&lbl("hg_sx"));
    ld_rr(asm, Reg8::C, Reg8::A);
    ld16(asm, Reg16Data::HL, l.lut_lo_page + 0x200);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.label(&lbl("hg_fillp"));
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_fillp"));
    asm.label(&lbl("hg_filln"));
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_filln"));
    // accumulate: D = head page (0x40 + lane - lane_lo), E over 0..pg_len,
    // HL walks the WRAM logit page (3 bytes per page-local id).
    a_from(asm, LANE);
    if lane_lo > 0 {
        asm.i(Instr::SubA {
            src: AluSrc8::Imm(lane_lo as u8),
        });
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_r_imm(asm, Reg8::E, 0);
    ld16(asm, Reg16Data::HL, l.logits);
    asm.label(&lbl("hg_acc"));
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, lut_hi);
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AddA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AdcA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::AdcA {
        src: AluSrc8::HlIndirect,
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::E),
    });
    // loop while E != pg_len (dynamic per-page bound). Save E in B, load
    // pg_len into A, CP A,B; B is reloaded to lut_hi at the loop top.
    ld_rr(asm, Reg8::B, Reg8::E);
    a_from(asm, pg.pg_len);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), &lbl("hg_acc"));
    a_from(asm, LANE);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(lane_hi as u8),
    });
    asm.jp(Some(Cond::NZ), &lbl("hg_lane"));
    asm.i(Instr::Ret { cond: None });
}

/// `rng_step`: advance the XorShift16 state at [`S_RNG_ADDR`] by one step
/// of the pinned (7, 9, 8) triple: `x ^= x << 7; x ^= x >> 9; x ^= x << 8`
/// (byte-serial shifts on the 2-byte LE state).
fn emit_rng_step(asm: &mut ModelAsm) {
    asm.label("rng_step");
    mem_copy(asm, SMP_RT, S_RNG_ADDR, 2);
    for _ in 0..7 {
        mem_shl1(asm, SMP_RT, 2);
    }
    xor_mem(asm, S_RNG_ADDR, SMP_RT, 2);
    mem_copy(asm, SMP_RT, S_RNG_ADDR, 2);
    for _ in 0..9 {
        mem_shr1(asm, SMP_RT, 2);
    }
    xor_mem(asm, S_RNG_ADDR, SMP_RT, 2);
    // x ^= x << 8  is  hi ^= lo.
    a_from(asm, S_RNG_ADDR);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, S_RNG_ADDR + 1);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_to(asm, S_RNG_ADDR + 1);
    asm.i(Instr::Ret { cond: None });
}

/// `smp_weight`: SMP_D (u24 logit deficit) -> A = exp-LUT weight.
/// Mirrors `gbf_kernel::decode` exactly.
fn emit_smp_weight(asm: &mut ModelAsm, scale_q16: u16) {
    asm.label("smp_weight");
    // SMP_P = lo16(d) * scale (u32) with a fifth zero byte.
    a_from(asm, SMP_D);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, SMP_D + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    ld16(asm, Reg16Data::DE, scale_q16);
    asm.call("mul16"); // MUL_R = BC * DE, DE preserved
    mem_copy(asm, SMP_P, crate::asm_impl_model::MUL_R, 4);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, SMP_P + 4);
    // += (hi8(d) * scale) << 16
    a_from(asm, SMP_D + 2);
    asm.call("mul16x8"); // C:HL = hi8 * scale
    a_from(asm, SMP_P + 2);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, SMP_P + 2);
    a_from(asm, SMP_P + 3);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::H),
    });
    a_to(asm, SMP_P + 3);
    a_from(asm, SMP_P + 4);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, SMP_P + 4);
    // round-half-up: += 0x8000
    a_from(asm, SMP_P + 1);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, SMP_P + 1);
    carry_ripple(asm, SMP_P + 2, 3);
    // u = min(255, P >> 16)
    a_from(asm, SMP_P + 3);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_P + 4);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "smw_sat");
    a_from(asm, SMP_P + 2);
    asm.jr(None, "smw_lut");
    asm.label("smw_sat");
    ld_r_imm(asm, Reg8::A, 255);
    asm.label("smw_lut");
    // A = exp_lut[u]
    ld_rr(asm, Reg8::B, Reg8::A);
    asm.ld16_label(Reg16Data::HL, "exp_lut", 0);
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::Ret { cond: None });
}

/// `sample_v`: the integer top-k/temperature sampling epilogue over the
/// `vocab` i24 logits. Must run right after `argmax_v` (candidate 0 = the
/// argmax with LUT weight `exp_lut[0]`). Byte-exact mirror of
/// `gbf_kernel::decode::sample_topk`.
fn emit_sample_topk(asm: &mut ModelAsm, l: &StateWramLayout, k: u8) {
    let vocab = l.topology.vocab;
    let used_lo = (l.samp_used & 0xFF) as u8;
    let used_hi = (l.samp_used >> 8) as u8;
    let ids_lo = (l.samp_ids & 0xFF) as u8;
    let wts_lo = (l.samp_wts & 0xFF) as u8;
    let logits_hi = (l.logits >> 8) as u8;
    debug_assert_eq!(l.logits & 0xFF, 0, "logits page-aligned");
    use crate::asm_impl_model::ARG_CAND;

    asm.label("sample_v");
    // clear the vocab used flags
    ld16(asm, Reg16Data::HL, l.samp_used);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_r_imm(asm, Reg8::B, vocab as u8);
    asm.label("smp_clr");
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "smp_clr");

    // candidate 0 = argmax (d = 0 -> u = 0 -> w = exp_lut[0])
    mem_copy(asm, SMP_M, crate::asm_impl_model::ARG_BEST, 3);
    a_from(asm, S_ARGMAX_ADDR);
    a_to(asm, l.samp_ids);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(used_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    ld_r_imm(asm, Reg8::A, 1);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.ld16_label(Reg16Data::HL, "exp_lut", 0);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(asm, l.samp_wts);
    a_to(asm, SMP_TOT);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, SMP_TOT + 1);
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SMP_PASS);

    // passes 1..k
    asm.label("smp_pass");
    a_from(asm, SMP_PASS);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(k),
    });
    asm.jp(Some(Cond::Z), "smp_draw");
    ld_r_imm(asm, Reg8::A, 0xFF);
    a_to(asm, SMP_BESTID);
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("smp_scan");
    // skip used ids
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(used_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::NZ), "smp_next");
    // load logit id C (3 bytes at logits + 3*C), sign-flip hi
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, logits_hi);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::XorA {
        src: AluSrc8::Imm(0x80),
    });
    a_to(asm, ARG_CAND + 2);
    // first unused candidate is taken unconditionally
    a_from(asm, SMP_BESTID);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0xFF),
    });
    asm.jr(Some(Cond::Z), "smp_take");
    // 3-byte unsigned compare: take iff CAND > BEST
    for kb in [2u16, 1, 0] {
        a_from(asm, ARG_CAND + kb);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, SMP_BEST + kb);
        asm.i(Instr::CpA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::C), "smp_take");
        if kb > 0 {
            asm.jp(Some(Cond::NZ), "smp_next");
        } else {
            asm.jp(None, "smp_next");
        }
    }
    asm.label("smp_take");
    mem_copy(asm, SMP_BEST, ARG_CAND, 3);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, SMP_BESTID);
    asm.label("smp_next");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(vocab as u8),
    });
    asm.jp(Some(Cond::NZ), "smp_scan");
    // commit the pass winner: mark used, record id
    a_from(asm, SMP_BESTID);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(used_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    ld_r_imm(asm, Reg8::A, 1);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    a_from(asm, SMP_PASS);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(ids_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    a_from(asm, SMP_BESTID);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    // d = M - best (u24; candidates are visited in descending order)
    mem_sub_into(asm, SMP_D, SMP_M, SMP_BEST, 3);
    asm.call("smp_weight"); // A = w
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, SMP_PASS);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(wts_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    // total += w
    a_from(asm, SMP_TOT);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, SMP_TOT);
    a_from(asm, SMP_TOT + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, SMP_TOT + 1);
    a_from(asm, SMP_PASS);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, SMP_PASS);
    asm.jp(None, "smp_pass");

    // draw: r = rng_step(); threshold = (r * total) >> 16
    asm.label("smp_draw");
    asm.call("rng_step");
    a_from(asm, S_RNG_ADDR);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, S_RNG_ADDR + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_TOT);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, SMP_TOT + 1);
    ld_rr(asm, Reg8::D, Reg8::A);
    asm.call("mul16"); // MUL_R = r * total (u32)
    mem_copy(asm, SMP_THR, crate::asm_impl_model::MUL_R + 2, 2);
    zero_mem(asm, SMP_CUM, 2);
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("smp_walk");
    // cum += wts[j]
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(wts_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_CUM);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_to(asm, SMP_CUM);
    a_from(asm, SMP_CUM + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, SMP_CUM + 1);
    // borrow on threshold - cum  <=>  cum > threshold  ->  pick
    a_from(asm, SMP_CUM);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_THR);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_from(asm, SMP_CUM + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_THR + 1);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::C), "smp_pick");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(k),
    });
    asm.jp(Some(Cond::NZ), "smp_walk");
    // structurally unreachable (threshold < total); defensively pick last
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.label("smp_pick");
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(ids_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(asm, S_SAMPLED_ADDR);
    asm.i(Instr::Ret { cond: None });
}

/// `argmax_fold_pg`: fold the current output-page's `pg_len` logits (at
/// [`StateWramLayout::logits`], page-local ids) into the running top-1 at
/// `paged.argmax16` / `paged.best_logit`. Ascending page-local ids, strict `>`
/// (lowest global id wins ties across pages, because pages and page-local ids
/// both ascend). `best_logit` is seeded to the i24 minimum by the driver, so
/// the very first candidate always wins — exactly the host's `!seen_any` rule.
/// The raw i24 best logit is stored at `paged.best_logit`; the sign flip is
/// transient (in the compare registers only).
fn emit_argmax_fold_pg(asm: &mut ModelAsm, l: &StateWramLayout) {
    use crate::asm_impl_model::ARG_CAND;
    let pg = l.paged.expect("paged layout");
    let logits_hi = (l.logits >> 8) as u8;
    debug_assert_eq!(l.logits & 0xFF, 0, "logits page-aligned");
    asm.label("argmax_fold_pg");
    // C = page-local id (0..pg_len)
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("axf_loop");
    // load candidate raw i24 from logits + 3*C
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, logits_hi);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, ARG_CAND + 2);
    // ARG_BEST = best_logit (raw); compare sign-flipped copies in registers.
    // 3-byte signed compare: take iff CAND > BEST.
    for k in [2u16, 1, 0] {
        a_from(asm, ARG_CAND + k);
        if k == 2 {
            asm.i(Instr::XorA {
                src: AluSrc8::Imm(0x80),
            });
        }
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, pg.best_logit + k);
        if k == 2 {
            asm.i(Instr::XorA {
                src: AluSrc8::Imm(0x80),
            });
        }
        asm.i(Instr::CpA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::C), "axf_upd");
        if k > 0 {
            asm.jr(Some(Cond::NZ), "axf_next");
        } else {
            asm.jr(None, "axf_next");
        }
    }
    asm.label("axf_upd");
    // best_logit := candidate (raw i24)
    mem_copy(asm, pg.best_logit, ARG_CAND, 3);
    // argmax16 := pg_base + C
    a_from(asm, pg.pg_base);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, pg.argmax16);
    a_from(asm, pg.pg_base + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, pg.argmax16 + 1);
    asm.label("axf_next");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    a_from(asm, pg.pg_len);
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "axf_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `heap_offer_pg`: offer each of the current output-page's `pg_len` logits to
/// the running top-k heap (`paged.heap_logit` / `paged.heap_id`, count at
/// `paged.heap_count`), retaining the `k` entries best under the sampler total
/// order (logit desc, id asc). The heap is kept INSERTION-SORTED ascending in
/// selection order (worst at index 0), mirroring `RunningTopK::insert_sorted`,
/// so eviction pops index 0 and finalize is a pure reversal. `k` is the
/// compile-time heap size (`min(HEAP_K_MAX, vocab)`).
fn emit_heap_offer_pg(asm: &mut ModelAsm, l: &StateWramLayout, k: u8) {
    let pg = l.paged.expect("paged layout");
    let logits_hi = (l.logits >> 8) as u8;
    asm.label("heap_offer_pg");
    // C = page-local id (0..pg_len)
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("ho_loop");
    // cand_logit (raw i24) := logits + 3*C
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, logits_hi);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.cand_logit);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.cand_logit + 1);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.cand_logit + 2);
    // cand_id := pg_base + C (u16)
    a_from(asm, pg.pg_base);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, pg.cand_id);
    a_from(asm, pg.pg_base + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, pg.cand_id + 1);
    // save C (page-local id) across the offer call
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::Push {
        src: Reg16Stack::BC,
    });
    asm.call("heap_offer_one");
    asm.i(Instr::Pop {
        dst: Reg16Stack::BC,
    });
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    a_from(asm, pg.pg_len);
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "ho_loop");
    asm.i(Instr::Ret { cond: None });

    // heap_offer_one: offer (cand_logit, cand_id) to the sorted heap.
    emit_heap_offer_one(asm, l, k);
}

/// `heap_offer_one`: the [`RunningTopK::offer`] step. If the heap is not full,
/// insert the candidate keeping ascending selection order (worst at 0). If
/// full, replace slot 0 (the worst) with the candidate iff the candidate ranks
/// above it, then re-sort by sifting up. Uses `worst_idx`/`worst_logit` as
/// scratch. `k` is the heap capacity.
fn emit_heap_offer_one(asm: &mut ModelAsm, l: &StateWramLayout, k: u8) {
    let pg = l.paged.expect("paged layout");
    asm.label("heap_offer_one");
    // if count < k -> append then sift up.  else compare with slot 0 (worst).
    a_from(asm, pg.heap_count);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(k),
    });
    asm.jr(Some(Cond::NZ), "ho1_append");
    // full: cand vs slot 0 (worst). take iff ranks_above(cand, heap[0]).
    // ranks_above(a,b): a.logit > b.logit || (a.logit==b.logit && a.id<b.id)
    // heap[0] worst is at heap_logit+0 / heap_id+0.
    asm.call("heap_cmp_cand_slot0"); // carry set iff cand ranks ABOVE slot 0
    asm.jr(Some(Cond::C), "ho1_replace0");
    asm.i(Instr::Ret { cond: None });
    asm.label("ho1_replace0");
    // overwrite slot 0 (the worst) with cand, then sift FORWARD (toward higher
    // indices) while cand ranks above its successor — restoring ascending order
    // (worst at 0). worst_idx is the forward-sift cursor, starting at 0.
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, pg.worst_idx);
    asm.call("heap_write_slot_cand"); // heap[0] := cand
    asm.label("ho1_fwd");
    // stop if worst_idx == count-1 (no successor).
    a_from(asm, pg.heap_count);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, pg.worst_idx);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::Z), "ho1_done");
    // swap iff heap[worst_idx] ranks ABOVE heap[worst_idx+1] (out of order).
    asm.call("heap_cmp_fwd");
    asm.jr(Some(Cond::C), "ho1_fwd_swap");
    asm.i(Instr::Ret { cond: None });
    asm.label("ho1_fwd_swap");
    asm.call("heap_swap_fwd");
    a_from(asm, pg.worst_idx);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, pg.worst_idx);
    asm.jr(None, "ho1_fwd");

    asm.label("ho1_append");
    // heap[count] := cand; worst_idx := count; count += 1; sift up toward 0.
    a_from(asm, pg.heap_count);
    a_to(asm, pg.worst_idx);
    asm.call("heap_write_slot_cand");
    a_from(asm, pg.heap_count);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, pg.heap_count);
    asm.label("ho1_sift");
    // sift up: while worst_idx>0 and heap[worst_idx-1] ranks_above heap[worst_idx], swap.
    a_from(asm, pg.worst_idx);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "ho1_done");
    asm.call("heap_cmp_up"); // carry set iff heap[i-1] ranks ABOVE heap[i]
    asm.jr(Some(Cond::C), "ho1_swap");
    asm.i(Instr::Ret { cond: None });
    asm.label("ho1_swap");
    asm.call("heap_swap_up"); // swap heap[i] and heap[i-1]
    a_from(asm, pg.worst_idx);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, pg.worst_idx);
    asm.jr(None, "ho1_sift");
    asm.label("ho1_done");
    asm.i(Instr::Ret { cond: None });

    // --- heap helper routines ---
    emit_heap_helpers(asm, l);
}

/// The low-level heap helpers shared by [`emit_heap_offer_one`]:
/// `heap_write_slot_cand` (heap[worst_idx] := cand), `heap_cmp_cand_slot0`
/// (carry iff cand ranks above heap[0]), `heap_cmp_up` (carry iff heap[i] ranks
/// above heap[i-1]), `heap_swap_up` (swap heap[i], heap[i-1]). All indices are
/// u8 slot numbers; logits are i24 (3 B), ids u16 (2 B). "ranks above" is
/// logit-desc / id-asc, computed as a signed 3-byte logit compare with a u16
/// id tiebreak.
fn emit_heap_helpers(asm: &mut ModelAsm, l: &StateWramLayout) {
    let pg = l.paged.expect("paged layout");
    // heap_write_slot_cand: heap[worst_idx] := (cand_logit, cand_id).
    asm.label("heap_write_slot_cand");
    // HL = heap_logit + 3*worst_idx
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        a_from(asm, pg.cand_logit + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    // HL = heap_id + 2*worst_idx
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        a_from(asm, pg.cand_id + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    asm.i(Instr::Ret { cond: None });

    // heap_cmp_cand_slot0: carry := ranks_above(cand, heap[0]).
    asm.label("heap_cmp_cand_slot0");
    // load heap[0] logit -> worst_logit, id -> (worst scratch via regs).
    // Compare cand_logit vs heap[0].logit (3-byte signed). Then tiebreak id.
    heap_load_slot_logit(asm, pg, 0u8);
    // compare cand vs worst_logit as ranks_above
    heap_ranks_above_cand(asm, pg, /*slot=*/ 0);
    asm.i(Instr::Ret { cond: None });

    // heap_cmp_up: carry := ranks_above(heap[worst_idx], heap[worst_idx-1]).
    asm.label("heap_cmp_up");
    heap_ranks_above_slots(asm, pg);
    asm.i(Instr::Ret { cond: None });

    // heap_swap_up: swap heap[worst_idx] and heap[worst_idx-1] (logit+id).
    asm.label("heap_swap_up");
    heap_swap_up_body(asm, pg);
    asm.i(Instr::Ret { cond: None });

    // heap_cmp_fwd: carry := ranks_above(heap[worst_idx], heap[worst_idx+1]).
    asm.label("heap_cmp_fwd");
    heap_ranks_above_fwd(asm, pg);
    asm.i(Instr::Ret { cond: None });

    // heap_swap_fwd: swap heap[worst_idx] and heap[worst_idx+1] (logit+id).
    asm.label("heap_swap_fwd");
    heap_swap_fwd_body(asm, pg);
    asm.i(Instr::Ret { cond: None });
}

/// carry := ranks_above(heap[worst_idx], heap[worst_idx+1]): stage cur into
/// cand, next into worst/best scratch, compute ranks_above(cur, next).
fn heap_ranks_above_fwd(asm: &mut ModelAsm, pg: PagedSampler) {
    // cur = heap[worst_idx] -> cand_logit/cand_id
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_logit + k);
    }
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_id + k);
    }
    // next = heap[worst_idx+1] -> worst_logit (logit) / heap_scratch_id (id)
    next_slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.worst_logit + k);
    }
    next_slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.heap_scratch_id);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.heap_scratch_id + 1);
    ranks_above_3b_then_id(
        asm,
        pg.cand_logit,
        pg.worst_logit,
        pg.cand_id,
        pg.heap_scratch_id,
    );
}

/// swap heap[worst_idx] and heap[worst_idx+1] (logit 3 B, id 2 B).
fn heap_swap_fwd_body(asm: &mut ModelAsm, pg: PagedSampler) {
    // stage cur logit -> cand_logit, next logit -> worst_logit, write swapped.
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_logit + k);
    }
    next_slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.worst_logit + k);
    }
    next_slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        a_from(asm, pg.cand_logit + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        a_from(asm, pg.worst_logit + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    // id swap
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_id + k);
    }
    next_slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.heap_scratch_id + k);
    }
    next_slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        a_from(asm, pg.cand_id + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        a_from(asm, pg.heap_scratch_id + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
}

/// HL := base + stride * (`idx` + 1).
fn next_slot_ptr(asm: &mut ModelAsm, base: u16, idx_addr: u16, stride: u8) {
    a_from(asm, idx_addr);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    match stride {
        2 => {
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
        }
        3 => {
            ld_rr(asm, Reg8::B, Reg8::A);
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::B),
            });
        }
        _ => unreachable!("heap strides are 2 or 3"),
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((base & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (base >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
}

/// HL := base + stride * (`idx`), where `idx` is a u8 at WRAM `idx_addr`.
fn slot_ptr(asm: &mut ModelAsm, base: u16, idx_addr: u16, stride: u8) {
    a_from(asm, idx_addr);
    // A := idx * stride (stride in {2,3}); use adds.
    match stride {
        2 => {
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
        }
        3 => {
            ld_rr(asm, Reg8::B, Reg8::A);
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::B),
            });
        }
        _ => unreachable!("heap strides are 2 or 3"),
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((base & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (base >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
}

/// Stage heap[`slot`].logit (3 B, raw i24) into `worst_logit`. The slot's id
/// is read on demand by the comparator (`heap_id + 2*slot`), so this function
/// only stages the LOGIT. `slot` is a small immediate.
fn heap_load_slot_logit(asm: &mut ModelAsm, pg: PagedSampler, slot: u8) {
    // HL = heap_logit + 3*slot (slot is a small immediate here)
    let addr = pg.heap_logit + 3 * u16::from(slot);
    ld16(asm, Reg16Data::HL, addr);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.worst_logit + k);
    }
}

/// carry := ranks_above(cand, heap[`slot`]) where the slot logit is already in
/// `worst_logit`. logit-desc then id-asc. `slot` is a small immediate id
/// address `heap_id + 2*slot`.
fn heap_ranks_above_cand(asm: &mut ModelAsm, pg: PagedSampler, slot: u16) {
    // 3-byte signed compare cand_logit vs worst_logit.
    // carry-out contract: set carry iff cand ranks strictly above.
    let id_addr = pg.heap_id + 2 * slot;
    ranks_above_3b_then_id(asm, pg.cand_logit, pg.worst_logit, pg.cand_id, id_addr);
}

/// carry := ranks_above(heap[worst_idx], heap[worst_idx-1]).
fn heap_ranks_above_slots(asm: &mut ModelAsm, pg: PagedSampler) {
    // Stage heap[worst_idx] logit into cand_logit, id into cand_id; stage
    // heap[worst_idx-1] logit into worst_logit; id compared via pointer.
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_logit + k);
    }
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_id + k);
    }
    // worst_idx-1 into a temp: we need heap[worst_idx-1]. Compute (worst_idx-1)
    // pointer by loading worst_idx, dec, into a scratch byte reuse of pg.heap_count?
    // Use pg.worst_logit region for the previous logit; previous id via pointer.
    // Build previous-slot logit pointer.
    a_from(asm, pg.worst_idx);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    // A = prev idx; HL = heap_logit + 3*prev
    ld_rr(asm, Reg8::B, Reg8::A);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.heap_logit & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.heap_logit >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.worst_logit + k);
    }
    // previous id pointer: heap_id + 2*(worst_idx-1) -> store into best_logit
    // scratch (2 bytes) to compare against cand_id.
    a_from(asm, pg.worst_idx);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.heap_id & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.heap_id >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.heap_scratch_id); // prev id lo
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.heap_scratch_id + 1); // prev id hi
    // Ascending heap (worst at 0): swap on sift-up iff the PREDECESSOR
    // heap[i-1] ranks ABOVE the current heap[i], i.e. heap[i] is out of place
    // below a better entry. So compute ranks_above(prev, cur):
    //   a = prev = (worst_logit, heap_scratch_id),  b = cur = (cand_logit, cand_id).
    ranks_above_3b_then_id(
        asm,
        pg.worst_logit,
        pg.cand_logit,
        pg.heap_scratch_id,
        pg.cand_id,
    );
}

/// carry := ranks_above((`a_logit`,`a_id`), (`b_logit`,`b_id_addr`)):
/// a.logit > b.logit  OR  (a.logit == b.logit  AND  a.id < b.id).
/// All logits are raw i24 (3 B LE); ids u16 (2 B LE). Leaves carry set iff a
/// ranks strictly above b; clears carry otherwise.
fn ranks_above_3b_then_id(asm: &mut ModelAsm, a_logit: u16, b_logit: u16, a_id: u16, b_id: u16) {
    // Signed 3-byte compare of a_logit vs b_logit. Determine >, <, or ==.
    // We compute a - b (as signed via sign-flip on top byte) from the top byte
    // down. Result flags: if a > b -> set carry (rank above), ret. if a < b ->
    // clear carry, ret. if equal -> fall to id tiebreak.
    let lbl = asm.fresh("rka");
    let above = format!("{lbl}_above");
    let below = format!("{lbl}_below");
    let idtb = format!("{lbl}_id");
    let done = format!("{lbl}_done");
    for k in [2u16, 1, 0] {
        // B := b byte (top byte sign-flipped)
        a_from(asm, b_logit + k);
        if k == 2 {
            asm.i(Instr::XorA {
                src: AluSrc8::Imm(0x80),
            });
        }
        ld_rr(asm, Reg8::B, Reg8::A);
        // A := a byte (top byte sign-flipped)
        a_from(asm, a_logit + k);
        if k == 2 {
            asm.i(Instr::XorA {
                src: AluSrc8::Imm(0x80),
            });
        }
        asm.i(Instr::CpA {
            src: AluSrc8::Reg(Reg8::B),
        });
        // A < B (carry) => a_byte < b_byte => a below at this byte => a<b.
        asm.jr(Some(Cond::C), &below);
        // A != B and not carry => a_byte > b_byte => a>b.
        asm.jr(Some(Cond::NZ), &above);
        // equal: continue to next byte
    }
    // all three equal -> tiebreak on id (a.id < b.id -> above)
    asm.jr(None, &idtb);
    asm.label(&above);
    asm.i(Instr::Scf); // set carry = rank above
    asm.jr(None, &done);
    asm.label(&below);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    // clear carry: A xor A sets Z, clears carry.
    asm.jr(None, &done);
    asm.label(&idtb);
    // u16 compare a.id < b.id : compute a.id - b.id, carry iff a<b.
    a_from(asm, b_id);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, a_id);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_from(asm, b_id + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, a_id + 1);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    // carry set iff a.id < b.id -> rank above. Leave carry as-is.
    asm.label(&done);
}

/// swap heap[worst_idx] and heap[worst_idx-1] (both logit 3 B and id 2 B),
/// byte-wise through the A register.
fn heap_swap_up_body(asm: &mut ModelAsm, pg: PagedSampler) {
    // logit swap
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    // stage cur logit into cand_logit
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_logit + k);
    }
    // prev logit pointer
    prev_slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.worst_logit + k);
    }
    // write cur->prev
    prev_slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        a_from(asm, pg.cand_logit + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    // write prev->cur
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        a_from(asm, pg.worst_logit + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    // id swap (2 B), stage into cand_id / heap_scratch_id scratch
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_id + k);
    }
    prev_slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.heap_scratch_id + k);
    }
    prev_slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        a_from(asm, pg.cand_id + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    for k in 0..2u16 {
        a_from(asm, pg.heap_scratch_id + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
}

/// HL := base + stride * (`idx` - 1).
fn prev_slot_ptr(asm: &mut ModelAsm, base: u16, idx_addr: u16, stride: u8) {
    a_from(asm, idx_addr);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    match stride {
        2 => {
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
        }
        3 => {
            ld_rr(asm, Reg8::B, Reg8::A);
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            });
            asm.i(Instr::AddA {
                src: AluSrc8::Reg(Reg8::B),
            });
        }
        _ => unreachable!("heap strides are 2 or 3"),
    }
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((base & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (base >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
}

/// `sample_paged`: finalize the running top-k heap and draw one token
/// (`S_SAMPLED_ADDR`). Byte-identical to
/// `decode::sample_topk_from_candidates` on the host's finalized heap.
///
/// The heap is insertion-sorted ascending (worst at 0), so selection order
/// (best first) is the reversed heap: candidate `j` = heap slot `count-1-j`.
/// For each candidate `j`, `d = logit_max - logit_j` (u24) with
/// `logit_max = heap_logit[count-1]`, weight = `smp_weight(d)`, and
/// `samp_ids[j]` (u16) / `samp_wts[j]` (u8) are filled. Then draw: `r =
/// rng_step()`, `threshold = (r * total) >> 16`, cumulative walk picks the
/// first candidate with `cum > threshold`. Uses SMP_* scratch (shared with the
/// single-page sampler; only one is emitted per ROM).
fn emit_sample_paged(asm: &mut ModelAsm, l: &StateWramLayout) {
    let pg = l.paged.expect("paged layout");
    asm.label("sample_paged");
    // logit_max := heap_logit[count-1]  -> SMP_M (raw i24, used for deficit)
    a_from(asm, pg.heap_count);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    a_to(asm, pg.worst_idx); // reuse worst_idx as the top-slot index
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, SMP_M + k);
    }
    // total := 0
    zero_mem(asm, SMP_TOT, 2);
    // j := 0 ; heap cursor slot := count-1 (descending)
    ld_r_imm(asm, Reg8::C, 0); // C = candidate index j
    asm.label("spg_fill");
    ld_rr(asm, Reg8::A, Reg8::C);
    a_from(asm, pg.heap_count);
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::Z), "spg_draw");
    // slot := count-1-j  -> worst_idx
    a_from(asm, pg.heap_count);
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::C),
    });
    a_to(asm, pg.worst_idx);
    // samp_ids[j] := heap_id[slot]  (u16)
    slot_ptr(asm, pg.heap_id, pg.worst_idx, 2);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.cand_id);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, pg.cand_id + 1);
    // write samp_ids[j] = samp_ids + 2*j
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.samp_ids & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.samp_ids >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    a_from(asm, pg.cand_id);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    a_from(asm, pg.cand_id + 1);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    // d := SMP_M - heap_logit[slot]  (u24)  -> SMP_D
    slot_ptr(asm, pg.heap_logit, pg.worst_idx, 3);
    for k in 0..3u16 {
        asm.i(Instr::LdAFromReg16Addr {
            src: Reg16Addr::Hli,
        });
        a_to(asm, pg.cand_logit + k);
    }
    mem_sub_into(asm, SMP_D, SMP_M, pg.cand_logit, 3);
    // save C across the smp_weight call
    asm.i(Instr::Push {
        src: Reg16Stack::BC,
    });
    asm.call("smp_weight"); // A = w
    asm.i(Instr::Pop {
        dst: Reg16Stack::BC,
    });
    ld_rr(asm, Reg8::E, Reg8::A); // E = w (survives; B/C used below carefully)
    // samp_wts[j] := w  (samp_wts + j)
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.samp_wts & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.samp_wts >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    // total += w
    a_from(asm, SMP_TOT);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    a_to(asm, SMP_TOT);
    a_from(asm, SMP_TOT + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, SMP_TOT + 1);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.jp(None, "spg_fill");

    // draw: r = rng_step(); threshold = (r * total) >> 16
    asm.label("spg_draw");
    asm.call("rng_step");
    a_from(asm, S_RNG_ADDR);
    ld_rr(asm, Reg8::C, Reg8::A);
    a_from(asm, S_RNG_ADDR + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_TOT);
    ld_rr(asm, Reg8::E, Reg8::A);
    a_from(asm, SMP_TOT + 1);
    ld_rr(asm, Reg8::D, Reg8::A);
    asm.call("mul16"); // MUL_R = r * total (u32)
    mem_copy(asm, SMP_THR, crate::asm_impl_model::MUL_R + 2, 2);
    zero_mem(asm, SMP_CUM, 2);
    ld_r_imm(asm, Reg8::C, 0); // C = candidate index
    asm.label("spg_walk");
    // cum += samp_wts[C]
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.samp_wts & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.samp_wts >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_CUM);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_to(asm, SMP_CUM);
    a_from(asm, SMP_CUM + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, SMP_CUM + 1);
    // pick iff cum > threshold  (borrow on threshold - cum)
    a_from(asm, SMP_CUM);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_THR);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::B),
    });
    a_from(asm, SMP_CUM + 1);
    ld_rr(asm, Reg8::B, Reg8::A);
    a_from(asm, SMP_THR + 1);
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::C), "spg_pick");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    // stop when C == heap_count (defensive; threshold < total guarantees a hit)
    ld_rr(asm, Reg8::A, Reg8::C);
    a_from(asm, pg.heap_count);
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jp(Some(Cond::NZ), "spg_walk");
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.label("spg_pick");
    // S_SAMPLED(:S_SAMPLED_HI) := samp_ids[C] (u16). The wide-vocab subword
    // feedback needs the FULL id; the render/id_bytes table indexes it. The low
    // byte alone drives the SinglePage-compatible charset ring accessor.
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((pg.samp_ids & 0xFF) as u8),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::A, (pg.samp_ids >> 8) as u8);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    a_to(asm, S_SAMPLED_ADDR);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(asm, S_SAMPLED_HI_ADDR);
    asm.i(Instr::Ret { cond: None });
}

// ---------------------------------------------------------------------------
// bank plan + params blob
// ---------------------------------------------------------------------------

/// Per-row scale/decay tables, packed into one dedicated ROM bank (mapped
/// at [`CHUNK_ENTRY`] whenever an epilogue or the state update needs them).
pub(crate) struct ParamsBlob {
    pub(crate) bytes: Vec<u8>,
    pub(crate) state_in_scales: u16,
    pub(crate) state_out_scales: u16,
    pub(crate) decay: u16,
    /// (up scales, down scales) offsets per block.
    pub(crate) blocks: Vec<(u16, u16)>,
}

/// How a stateful ROM emits its matvec weights.
///
/// `V3` (default) emits each weight as straight-line `add`/`sub` machine code
/// (~6.6 B/weight), one matvec per bank chunk. `V2Dispatch` packs each weight
/// as a base-81 dispatch index (~0.25 B/weight, ~26x denser) walked by a
/// shared bank-0 handler, laying the packed streams contiguously across banks.
/// Both are byte-exact against the canonical integer evaluator; see
/// `docs/design/v2-dispatch-stateful.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WeightLowering {
    /// Weights-as-code, straight-line add/sub (the deployable default).
    #[default]
    V3,
    /// Weights-as-data, threaded base-81 dispatch (dense; opt-in).
    V2Dispatch,
}

/// One V2 matvec's placement in the packed weight banks.
#[derive(Debug, Clone, Copy)]
pub(crate) struct V2MatvecPlacement {
    /// 9-bit start bank of this matvec's stream.
    pub(crate) start_bank: u16,
    /// Byte offset of the stream start within `start_bank` (0..BANK_BYTES).
    pub(crate) start_offset: u16,
    /// True when this matvec uses the wide (i24) segment sentinels.
    pub(crate) wide: bool,
}

/// Per-block MoE dispatch plan (deploy step 4). Present only when the topology
/// is MoE (`n_experts > 1`). For a MoE model the V2 stream enumeration is
/// `state_in`, then per block `for e in 0..n_experts { up_e, down_e }`, so the
/// linear `v2_placements` order is deterministic but the forward body dispatches
/// to `expert_placements[block][EXPERT_SEL]` at runtime instead of consuming the
/// linear stream. The dense (`Dense`/`n_experts == 1`) path never builds a
/// `MoePlan`, so it is byte-identical to the pre-MoE ROM.
pub(crate) struct MoePlan {
    /// ROM bank holding each block's fixed-point router tables (win_q, bin_q,
    /// wout_q, bout_q, little-endian at CHUNK_ENTRY).
    pub(crate) router_bank: Vec<usize>,
    /// ROM bank holding each block's concatenated expert up/down scale tables.
    pub(crate) scale_bank: Vec<usize>,
    /// Per-block dispatch-table bytes (`n_experts * 12`), appended after the
    /// router code in each router bank at label `moe_disp`.
    pub(crate) disp_data: Vec<Vec<u8>>,
    /// Per-block fixed-point router table bytes (`FixedRouter::param_bytes`),
    /// appended at label `moe_tables`.
    pub(crate) router_params: Vec<Vec<u8>>,
    /// Raw scale-table bank payloads (bank order matches `scale_bank`).
    pub(crate) scale_bank_data: Vec<Vec<u8>>,
    /// Router shape (uniform across blocks): rank, n_experts, d_model.
    pub(crate) rank: usize,
    pub(crate) n_experts: usize,
    pub(crate) d_model: usize,
}

/// Weight-chunk plan and bank numbering shared by every stateful ROM
/// variant (one-token, multi-token, sampling, interactive shell).
pub(crate) struct StateRomPlan {
    pub(crate) lowering: WeightLowering,
    pub(crate) layout: StateWramLayout,
    /// V3 weight code: one `Vec<u8>` chunk per bank, grouped per matvec. Empty
    /// under `V2Dispatch`.
    pub(crate) per_matvec_chunks: Vec<Vec<Vec<u8>>>,
    /// V2 packed weight banks (each up to `BANK_BYTES`), laid contiguously.
    /// Empty under `V3`.
    pub(crate) v2_weight_banks: Vec<Vec<u8>>,
    /// V2 per-matvec placement, in forward-pass call order. Empty under `V3`.
    pub(crate) v2_placements: Vec<V2MatvecPlacement>,
    pub(crate) weight_chunk_count: usize,
    pub(crate) weight_code_bytes: usize,
    pub(crate) params: ParamsBlob,
    pub(crate) params_bank: usize,
    pub(crate) state_bank0: usize,
    /// Rows served by each state-table bank, in bank order.
    pub(crate) state_bank_rows: Vec<u8>,
    pub(crate) state_stride: usize,
    pub(crate) emb_bank0: usize,
    pub(crate) emb_stride: usize,
    pub(crate) emb_bank_count: usize,
    pub(crate) head_bank0: usize,
    /// (lane_lo, lane_hi) per head bank.
    pub(crate) head_groups: Vec<(usize, usize)>,
    /// Number of streamed logit output-pages (`ceil(vocab / 85)` under Paged,
    /// else 1). Head banks are laid out as `n_logit_pages` sets, one per group.
    pub(crate) n_logit_pages: usize,
    /// MoE dispatch plan (`Some` only for a MoE topology, `n_experts > 1`).
    pub(crate) moe: Option<MoePlan>,
    /// Total banks including `extra_banks` appended after the head banks.
    pub(crate) bank_count: usize,
    /// When true, `chunk_run` calls a caller-emitted `anim_tick` routine once
    /// per weight chunk (SP is home between chunk calls). Only the interactive
    /// shell sets this and emits `anim_tick`; every other ROM leaves it false
    /// so `chunk_run` is byte-identical to the pre-animation emission.
    pub(crate) animate: bool,
}

impl StateRomPlan {
    /// Bank number of the first caller-owned `extra_banks` bank (the ones passed
    /// as `extra_bank_payloads` to [`assemble_state_rom`]), i.e. immediately
    /// after the weight/params/state/emb/head banks and any MoE router/scale
    /// banks. Mirrors the `extras_bank0` computation in `assemble_state_rom`.
    pub(crate) fn extras_bank0(&self) -> usize {
        let mut b = self.head_bank0 + self.head_groups.len() * self.n_logit_pages;
        if let Some(moe) = &self.moe {
            b += moe.router_bank.len() + moe.scale_bank.len();
        }
        b
    }
}

fn push_scales(bytes: &mut Vec<u8>, layer: &crate::model_ref::TernaryLayer) -> u16 {
    let off = bytes.len() as u16;
    for row in 0..layer.rows() {
        bytes.extend_from_slice(&layer.scale_raw(row).to_le_bytes());
    }
    off
}

/// Build the weight chunks/streams, params blob, and bank numbering: weight
/// banks 1..=W, the params bank, the state weight-table banks, the embedding
/// banks, the head banks, and finally `extra_banks` variant-owned banks. V3
/// keeps the exact pre-existing plan; V2 packs the base-81 dispatch streams
/// contiguously and numbers `ceil(total_stream_bytes / BANK_BYTES)` weight
/// banks.
pub(crate) fn plan_state_rom_with(
    model: &IntStateLoweredModel,
    layout: StateWramLayout,
    extra_banks: usize,
    lowering: WeightLowering,
) -> Result<StateRomPlan, ModelRomError> {
    let t = model.topology;
    let l = &layout;

    // Both lowerings enumerate matvecs in the same forward-pass order:
    // state_in, then (up, down) per block.
    let mut per_matvec_chunks: Vec<Vec<Vec<u8>>> = Vec::new();
    let mut v2_streams: Vec<(Vec<u8>, bool)> = Vec::new();
    match lowering {
        WeightLowering::V3 => {
            per_matvec_chunks.push(build_matvec_chunks_at(&model.state_in, l.act, l.acc)?);
            for (up, down) in &model.blocks {
                per_matvec_chunks.push(build_matvec_chunks_at(up, l.act, l.acc)?);
                match model.down_width {
                    AccWidth::I16 => {
                        per_matvec_chunks.push(build_matvec_chunks_at(down, l.act, l.acc)?);
                    }
                    AccWidth::I24 => {
                        per_matvec_chunks.push(build_matvec_chunks_wide(
                            &down.layer,
                            l.act,
                            l.acc,
                        )?);
                    }
                }
            }
        }
        WeightLowering::V2Dispatch => {
            v2_streams.push((build_matvec_stream_i16(&model.state_in)?, false));
            if t.is_moe() {
                // MoE V2 enumeration: state_in, then per block `for e in
                // 0..n_experts { up_e, down_e }`. The existing packing loop
                // below feeds all experts' streams contiguously; only WHICH
                // streams feed it changes. `moe_stream_index[block][e] =
                // (up_stream_idx, down_stream_idx)` maps back to placements.
                for block in &model.block_ffns {
                    let experts = match block {
                        crate::state_model_ref::LoweredBlockFfn::Moe { experts, .. } => experts,
                        crate::state_model_ref::LoweredBlockFfn::Dense { .. } => {
                            // A MoE topology must lower to all-MoE blocks
                            // (validated at lowering); defensive only.
                            return Err(ModelRomError::UnsupportedTopology {
                                detail: "MoE topology has a Dense lowered block".to_string(),
                            });
                        }
                    };
                    for (up, down) in experts {
                        v2_streams.push((build_matvec_stream_i16(up)?, false));
                        match model.down_width {
                            AccWidth::I16 => {
                                v2_streams.push((build_matvec_stream_i16(down)?, false));
                            }
                            AccWidth::I24 => {
                                v2_streams.push((build_matvec_stream_wide(&down.layer)?, true));
                            }
                        }
                    }
                }
            } else {
                for (up, down) in &model.blocks {
                    v2_streams.push((build_matvec_stream_i16(up)?, false));
                    match model.down_width {
                        AccWidth::I16 => {
                            v2_streams.push((build_matvec_stream_i16(down)?, false));
                        }
                        AccWidth::I24 => {
                            v2_streams.push((build_matvec_stream_wide(&down.layer)?, true));
                        }
                    }
                }
            }
        }
    }

    // Pack V2 streams contiguously into weight banks and record each matvec's
    // (start_bank, start_offset). Weight banks are numbered 1..=weight_banks.
    let mut v2_weight_banks: Vec<Vec<u8>> = Vec::new();
    let mut v2_placements: Vec<V2MatvecPlacement> = Vec::new();
    if lowering == WeightLowering::V2Dispatch {
        v2_weight_banks.push(Vec::with_capacity(BANK_BYTES));
        for (stream, wide) in &v2_streams {
            let start_bank = 1 + v2_weight_banks.len() as u16 - 1;
            let start_offset = v2_weight_banks.last().unwrap().len() as u16;
            v2_placements.push(V2MatvecPlacement {
                start_bank,
                start_offset,
                wide: *wide,
            });
            for &byte in stream {
                if v2_weight_banks.last().unwrap().len() == BANK_BYTES {
                    v2_weight_banks.push(Vec::with_capacity(BANK_BYTES));
                }
                v2_weight_banks.last_mut().unwrap().push(byte);
            }
        }
        // Drop a trailing empty bank if the last stream ended exactly on a
        // bank boundary (the loop only allocates the next bank lazily, so this
        // is defensive).
        if v2_weight_banks.last().is_some_and(Vec::is_empty) {
            v2_weight_banks.pop();
        }
    }

    let weight_chunk_count: usize = match lowering {
        WeightLowering::V3 => per_matvec_chunks.iter().map(Vec::len).sum(),
        WeightLowering::V2Dispatch => v2_weight_banks.len(),
    };
    let weight_code_bytes: usize = match lowering {
        WeightLowering::V3 => per_matvec_chunks
            .iter()
            .flat_map(|chunks| chunks.iter().map(Vec::len))
            .sum(),
        WeightLowering::V2Dispatch => v2_weight_banks.iter().map(Vec::len).sum(),
    };

    // Params blob (one bank).
    let mut bytes = Vec::new();
    let state_in_scales = push_scales(&mut bytes, &model.state_in.layer);
    let state_out_scales = push_scales(&mut bytes, &model.state_out);
    let decay = bytes.len() as u16;
    bytes.extend_from_slice(&model.decay_u8);
    // Dense `blocks` scale offsets: for a MoE model these hold expert 0's
    // scales (`model.blocks` is the dispatch-agnostic placeholder), never read
    // by the MoE forward body (which reads `moe.expert_scales` instead), but
    // kept populated so the params layout is uniform.
    let mut blocks = Vec::new();
    for (up, down) in &model.blocks {
        let up_off = push_scales(&mut bytes, &up.layer);
        let down_off = push_scales(&mut bytes, &down.layer);
        blocks.push((up_off, down_off));
    }

    if bytes.len() > BANK_BYTES {
        return Err(ModelRomError::ParamsBankOverflow { bytes: bytes.len() });
    }

    // MoE data banks: per block one ROUTER bank (the fixed-point router tables,
    // byte-for-byte `FixedRouter::param_bytes`, so host and ROM route from the
    // SAME bytes) and one SCALE bank (every expert's up/down per-row scale
    // tables concatenated). Both are mapped into the 0x4000 window on demand
    // (the router bank by `moe_router`, the scale bank by the up/down
    // epilogue). Each block's router/scale data must fit one bank; if a real
    // topology exceeds that, `ParamsBankOverflow` fires with the byte count.
    // The dense (`n_experts == 1`) path builds no MoE banks and is byte-identical.
    struct MoeData {
        router_params: Vec<Vec<u8>>,
        scale_banks: Vec<Vec<u8>>,
        expert_scale_off: Vec<Vec<(u16, u16)>>,
        rank: usize,
    }
    let moe_data = if t.is_moe() {
        if lowering != WeightLowering::V2Dispatch {
            return Err(ModelRomError::UnsupportedTopology {
                detail: "MoE topology requires V2 dispatch weight lowering".to_string(),
            });
        }
        let mut router_params: Vec<Vec<u8>> = Vec::with_capacity(t.n_blocks);
        let mut scale_banks: Vec<Vec<u8>> = Vec::with_capacity(t.n_blocks);
        let mut expert_scale_off: Vec<Vec<(u16, u16)>> = Vec::with_capacity(t.n_blocks);
        let mut rank = 0usize;
        for block in &model.block_ffns {
            let crate::state_model_ref::LoweredBlockFfn::Moe {
                experts,
                fixed_router,
                ..
            } = block
            else {
                return Err(ModelRomError::UnsupportedTopology {
                    detail: "MoE topology has a Dense lowered block".to_string(),
                });
            };
            rank = fixed_router.rank();
            // The router bank also carries the per-expert dispatch table (12
            // bytes/expert), prepended in the finalization step; check the total
            // fits one bank.
            let router = fixed_router.param_bytes();
            if router.len() + t.n_experts * 12 > BANK_BYTES {
                return Err(ModelRomError::ParamsBankOverflow {
                    bytes: router.len() + t.n_experts * 12,
                });
            }
            router_params.push(router);
            let mut scales = Vec::new();
            let mut per_expert = Vec::with_capacity(experts.len());
            for (up, down) in experts {
                let up_off = push_scales(&mut scales, &up.layer);
                let down_off = push_scales(&mut scales, &down.layer);
                per_expert.push((up_off, down_off));
            }
            if scales.len() > BANK_BYTES {
                return Err(ModelRomError::ParamsBankOverflow {
                    bytes: scales.len(),
                });
            }
            scale_banks.push(scales);
            expert_scale_off.push(per_expert);
        }
        Some(MoeData {
            router_params,
            scale_banks,
            expert_scale_off,
            rank,
        })
    } else {
        None
    };
    let params = ParamsBlob {
        bytes,
        state_in_scales,
        state_out_scales,
        decay,
        blocks,
    };

    // State out-projection table: power-of-two row stride, multi-bank.
    let state_stride = t.state_slots.next_power_of_two();
    if state_stride > BANK_BYTES {
        return Err(ModelRomError::TableRowTooWide {
            stride: state_stride,
        });
    }
    let state_rpb = BANK_BYTES / state_stride;
    let mut state_bank_rows = Vec::new();
    let mut remaining = t.d_model;
    while remaining > 0 {
        let rows = remaining.min(state_rpb);
        state_bank_rows.push(rows as u8);
        remaining -= rows;
    }

    // Embedding table: power-of-two stride >= 256 (single-page rows so the
    // in-bank address is just a page number).
    let emb_stride = (3 * t.d_model).next_power_of_two().max(256);
    if emb_stride > BANK_BYTES {
        return Err(ModelRomError::TableRowTooWide { stride: emb_stride });
    }
    let emb_rpb = BANK_BYTES / emb_stride;
    let emb_bank_count = t.vocab.div_ceil(emb_rpb);

    // Head: 64 lane pages of 256 bytes per bank.
    let head_groups: Vec<(usize, usize)> = (0..t.d_model)
        .step_by(64)
        .map(|lo| (lo, (lo + 64).min(t.d_model)))
        .collect();

    // Under Paged, each of the `n_logit_pages` output-pages gets its own set
    // of head banks (one per lane group), so a page's `<= LOGIT_PAGE_IDS` head
    // weights are addressable by page-local id 0..84 within a 256-byte lane
    // page. SinglePage keeps one head bank set (n_logit_pages == 1).
    let n_logit_pages = match t.logit_paging {
        crate::state_model_ref::LogitPaging::SinglePage => 1,
        crate::state_model_ref::LogitPaging::Paged => t
            .vocab
            .div_ceil(crate::state_model_ref::LOGIT_PAGE_IDS)
            .max(1),
    };

    let params_bank = 1 + weight_chunk_count;
    let state_bank0 = params_bank + 1;
    let emb_bank0 = state_bank0 + state_bank_rows.len();
    let head_bank0 = emb_bank0 + emb_bank_count;
    let head_bank_count = head_groups.len() * n_logit_pages;
    // MoE data banks come after the head banks: per block a router bank then a
    // scale bank (interleaved block by block).
    let moe_bank0 = head_bank0 + head_bank_count;

    // Finalize the MoE plan: derive `expert_placements[block][e]` from the
    // linear `v2_placements` (stream 0 = state_in, then per block per expert
    // (up, down)), and assign the router/scale bank numbers.
    let moe = moe_data.map(|d| {
        let n_experts = t.n_experts;
        let mut expert_placements =
            Vec::<Vec<(V2MatvecPlacement, V2MatvecPlacement)>>::with_capacity(t.n_blocks);
        for block in 0..t.n_blocks {
            let mut per_expert = Vec::with_capacity(n_experts);
            for e in 0..n_experts {
                let up_idx = 1 + (block * n_experts + e) * 2;
                let down_idx = up_idx + 1;
                per_expert.push((v2_placements[up_idx], v2_placements[down_idx]));
            }
            expert_placements.push(per_expert);
        }
        let mut router_bank = Vec::with_capacity(t.n_blocks);
        let mut scale_bank = Vec::with_capacity(t.n_blocks);
        for block in 0..t.n_blocks {
            router_bank.push(moe_bank0 + block * 2);
            scale_bank.push(moe_bank0 + block * 2 + 1);
        }
        // Per-block dispatch-table bytes (`MOE_DISP_ENTRY` = 14 bytes/expert):
        //   up_bank(2), up_bc(2), up_scale(2), down_bank(2), down_bc(2),
        //   down_scale(2), scale_bank(2)
        // The scale bank is per-block (same for all experts) but stored in each
        // entry so the shared `moe_up`/`moe_down` routines map it + set the
        // epilogue scale pointer without any per-block driver code. Appended
        // after the router code at `moe_disp`; the router tables follow at
        // `moe_tables`.
        let mut disp_data = Vec::with_capacity(t.n_blocks);
        for ((placements, scale_off), &sbank) in expert_placements
            .iter()
            .zip(d.expert_scale_off.iter())
            .zip(scale_bank.iter())
        {
            let mut bytes = Vec::with_capacity(n_experts * MOE_DISP_ENTRY);
            for (&(up, down), &(up_sc, down_sc)) in placements.iter().zip(scale_off.iter()) {
                let up_bc = CHUNK_ENTRY + up.start_offset;
                let down_bc = CHUNK_ENTRY + down.start_offset;
                let up_scale = CHUNK_ENTRY + up_sc;
                let down_scale = CHUNK_ENTRY + down_sc;
                bytes.extend_from_slice(&up.start_bank.to_le_bytes());
                bytes.extend_from_slice(&up_bc.to_le_bytes());
                bytes.extend_from_slice(&up_scale.to_le_bytes());
                bytes.extend_from_slice(&down.start_bank.to_le_bytes());
                bytes.extend_from_slice(&down_bc.to_le_bytes());
                bytes.extend_from_slice(&down_scale.to_le_bytes());
                bytes.extend_from_slice(&(sbank as u16).to_le_bytes());
            }
            disp_data.push(bytes);
        }
        MoePlan {
            router_bank,
            scale_bank,
            disp_data,
            router_params: d.router_params,
            scale_bank_data: d.scale_banks,
            rank: d.rank,
            n_experts,
            d_model: t.d_model,
        }
    });

    let moe_bank_count = moe.as_ref().map_or(0, |_| t.n_blocks * 2);
    let bank_count = moe_bank0 + moe_bank_count + extra_banks;
    if bank_count > 512 {
        return Err(ModelRomError::TooManyBanks { banks: bank_count });
    }
    Ok(StateRomPlan {
        lowering,
        layout,
        per_matvec_chunks,
        v2_weight_banks,
        v2_placements,
        weight_chunk_count,
        weight_code_bytes,
        params,
        params_bank,
        state_bank0,
        state_bank_rows,
        state_stride,
        emb_bank0,
        emb_stride,
        emb_bank_count,
        head_bank0,
        head_groups,
        n_logit_pages,
        moe,
        bank_count,
        animate: false,
    })
}

// ---------------------------------------------------------------------------
// top-level build
// ---------------------------------------------------------------------------

/// Emit the chunk-call run for one matvec: `n_chunks` consecutive banks
/// starting at `*next_bank`, dispatched through the shared `chunk_run`
/// loop routine ([`emit_chunk_run`]). The runs used to be unrolled at ~8
/// driver bytes per chunk; a real d192-scale checkpoint has hundreds of
/// chunks, which pushed the shell driver past the bank-0 window
/// ([`ModelRomError::DriverOverflowsBank0`]), so the loop form is the
/// scaling-correct emission (bd-pp43d).
fn emit_call_chunks(asm: &mut ModelAsm, n_chunks: usize, next_bank: &mut u16) {
    if n_chunks == 0 {
        return;
    }
    assert!(
        n_chunks <= 255,
        "one matvec's chunk run must fit the u8 loop counter (got {n_chunks})"
    );
    let bank = *next_bank;
    ld_r_imm(asm, Reg8::A, n_chunks as u8);
    a_to(asm, CHUNK_CNT);
    ld_r_imm(asm, Reg8::A, (bank & 0xFF) as u8);
    a_to(asm, CHUNK_BANK);
    ld_r_imm(asm, Reg8::A, (bank >> 8) as u8);
    a_to(asm, CHUNK_BANK + 1);
    asm.call("chunk_run");
    *next_bank += n_chunks as u16;
}

/// `chunk_run`: call [`CHUNK_ENTRY`] in [`CHUNK_CNT`] consecutive banks
/// starting at the 9-bit bank number in [`CHUNK_BANK`]. Both MBC5 bank
/// registers are rewritten every iteration; weight chunks clobber all
/// registers and repurpose SP, so the loop state lives in the fixed
/// scratch page the chunks never touch.
fn emit_chunk_run(asm: &mut ModelAsm, animate: bool) {
    let inc_a = |asm: &mut ModelAsm| {
        asm.i(Instr::Inc8 {
            dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
        });
    };
    asm.label("chunk_run");
    a_from(asm, CHUNK_BANK + 1);
    a_to(asm, MBC5_ROMB1);
    a_from(asm, CHUNK_BANK);
    a_to(asm, MBC5_ROMB0);
    asm.i(Instr::Call {
        cond: None,
        addr: CHUNK_ENTRY,
    });
    // The chunk restored SP before returning (call/ret), so between chunks SP is
    // home and every register is free (chunk_run reloads its loop state from
    // scratch WRAM). The shell uses this window to advance a per-chunk animation
    // tick; `anim_tick` writes only PPU registers (SCX/SCY/BGP), never VRAM, and
    // is emitted by the shell builder. No other ROM sets `animate`.
    if animate {
        asm.call("anim_tick");
    }
    // Advance the 9-bit bank number (lo wrap carries into the ROMB1 bit).
    a_from(asm, CHUNK_BANK);
    inc_a(asm);
    a_to(asm, CHUNK_BANK);
    asm.jr(Some(Cond::NZ), "chunk_run_nohi");
    a_from(asm, CHUNK_BANK + 1);
    inc_a(asm);
    a_to(asm, CHUNK_BANK + 1);
    asm.label("chunk_run_nohi");
    a_from(asm, CHUNK_CNT);
    asm.i(Instr::Dec8 {
        dst: gbf_asm::isa::IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, CHUNK_CNT);
    asm.jr(Some(Cond::NZ), "chunk_run");
    asm.i(Instr::Ret { cond: None });
}

// ---------------------------------------------------------------------------
// V2 dispatch: shared threaded-handler matvec routine
// ---------------------------------------------------------------------------
//
// A matvec's weights are a base-81 dispatch stream packed contiguously in the
// switchable-bank window (0x4000..0x8000). `matvec_v2`/`matvec_v2w` thread it,
// reproducing V3's per-row/segment accumulator `acc = bias + sum(+/- act)` mod
// 2^16 bit-for-bit and writing the SAME bytes to WRAM `l.acc` (design:
// docs/design/v2-dispatch-stateful.md). Register file during the walk:
//   DE = i16 accumulator     BC = stream pointer (0x4000..0x8000)
//   SP = activation pointer   HL/A = scratch (pop + dispatch + fetch)
//   CHUNK_BANK = 9-bit stream bank   SPSAVE = caller's return-address stack
//   WV2_OUT = output pointer   WV2_ACC = i24 wide-row accumulator
// Interrupts stay off for the whole ROM (the driver `di`s), as the V3 chunks
// already require, so SP-as-data is safe.

/// Emit one matvec's accumulation: V3 chunk-call run or a V2 dispatch call.
fn emit_state_matvec(
    asm: &mut ModelAsm,
    plan: &StateRomPlan,
    mv: &mut usize,
    chunk_iter: &mut std::slice::Iter<'_, Vec<Vec<u8>>>,
    next_bank: &mut u16,
) {
    match plan.lowering {
        WeightLowering::V3 => {
            let chunks = chunk_iter.next().expect("matvec chunks exist");
            emit_call_chunks(asm, chunks.len(), next_bank);
        }
        WeightLowering::V2Dispatch => {
            emit_v2_matvec_call(asm, plan.v2_placements[*mv]);
        }
    }
    *mv += 1;
}

/// Program the MBC5 bank + `CHUNK_BANK` + stream pointer for one V2 matvec and
/// call the shared handler routine.
fn emit_v2_matvec_call(asm: &mut ModelAsm, p: V2MatvecPlacement) {
    set_bank(asm, p.start_bank);
    ld_r_imm(asm, Reg8::A, (p.start_bank & 0xFF) as u8);
    a_to(asm, CHUNK_BANK);
    ld_r_imm(asm, Reg8::A, (p.start_bank >> 8) as u8);
    a_to(asm, CHUNK_BANK + 1);
    ld16(asm, Reg16Data::BC, CHUNK_ENTRY + p.start_offset);
    asm.call(if p.wide { "matvec_v2w" } else { "matvec_v2" });
}

/// Inline "fetch next stream byte": `A = *BC`, advance `BC`, and cross to the
/// next 9-bit bank (reprogramming MBC5, resetting `BC = 0x4000`) when the
/// pointer reaches 0x8000. Clobbers `A`, `HL`; preserves `DE`; leaves the byte
/// in `A`. Inlined (never `call`ed) because `SP` is the activation buffer
/// during the walk.
fn emit_v2_fetch(asm: &mut ModelAsm) {
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.i(Instr::Inc16 { dst: Reg16Data::BC });
    ld_rr(asm, Reg8::L, Reg8::A); // stash byte
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    let done = asm.fresh("v2fetch_done");
    asm.jr(Some(Cond::NZ), &done);
    // Cross bank: advance the 9-bit CHUNK_BANK (lo wrap carries into hi) and
    // reprogram MBC5 as we go. ROMB1 (hi bit) persists across bank switches, so
    // it only needs rewriting when the low byte wraps.
    a_from(asm, CHUNK_BANK);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, CHUNK_BANK);
    a_to(asm, MBC5_ROMB0); // A still holds the new low byte
    let nohi = asm.fresh("v2fetch_nohi");
    asm.jr(Some(Cond::NZ), &nohi);
    a_from(asm, CHUNK_BANK + 1);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, CHUNK_BANK + 1);
    a_to(asm, MBC5_ROMB1);
    asm.label(&nohi);
    ld16(asm, Reg16Data::BC, CHUNK_ENTRY); // reset pointer to 0x4000
    asm.label(&done);
    ld_rr(asm, Reg8::A, Reg8::L); // restore byte
}

/// `DE := next 2 stream bytes (LE)`.
fn emit_v2_bias_load(asm: &mut ModelAsm) {
    emit_v2_fetch(asm);
    ld_rr(asm, Reg8::E, Reg8::A);
    emit_v2_fetch(asm);
    ld_rr(asm, Reg8::D, Reg8::A);
}

/// `DE += reg` (unsigned byte, carry into D) — the branchless bake-off idiom.
fn emit_v2_add_de(asm: &mut ModelAsm, reg: Reg8) {
    ld_rr(asm, Reg8::A, reg);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(Reg8::E),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
}

/// `DE -= reg` (unsigned byte, borrow out of D). Computed from E directly so
/// no leading `ld a,reg` is needed: `a=e-reg`, then `d-=borrow`.
fn emit_v2_sub_de(asm: &mut ModelAsm, reg: Reg8) {
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::SubA {
        src: AluSrc8::Reg(reg),
    });
    ld_rr(asm, Reg8::E, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::SbcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
}

/// `HL := (ptr)` (2-byte LE WRAM pointer).
fn emit_load_hl_from(asm: &mut ModelAsm, ptr: u16) {
    a_from(asm, ptr);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, ptr + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
}

/// `(ptr) := HL`.
fn emit_store_hl_to(asm: &mut ModelAsm, ptr: u16) {
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, ptr);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, ptr + 1);
}

/// `SP := (SPSAVE); ret` — restore the caller's stack and return.
fn emit_v2_restore_sp_ret(asm: &mut ModelAsm) {
    emit_load_hl_from(asm, SPSAVE);
    asm.i(Instr::LdSpFromHl);
    asm.i(Instr::Ret { cond: None });
}

/// `WV2_ACC (i24) += sx24(DE)` — sign-extend the i16 segment partial and add
/// it byte-serially, exactly as `encode_row_wide` combines segments.
fn emit_v2_wide_combine(asm: &mut ModelAsm) {
    // L := 0x00/0xFF sign extension of D.
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::SbcA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    // Byte-serial add with carry (LD does not disturb the carry flag).
    a_from(asm, WV2_ACC);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    a_to(asm, WV2_ACC);
    a_from(asm, WV2_ACC + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    a_to(asm, WV2_ACC + 1);
    a_from(asm, WV2_ACC + 2);
    asm.i(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::L),
    });
    a_to(asm, WV2_ACC + 2);
}

/// Emit the shared V2 dispatch matvec routine, its 81 pattern handlers, the
/// five sentinel handlers, and the 86-entry handler table. Emitted once, in
/// bank 0, only when the plan's lowering is `V2Dispatch`.
fn emit_matvec_v2(asm: &mut ModelAsm, l: &StateWramLayout) {
    let act = l.act;
    let acc = l.acc;

    // --- entry points -------------------------------------------------------
    // i16 entry: save caller SP, seed out pointer + first bias, SP := act.
    asm.label("matvec_v2");
    asm.i(Instr::LdDirectFromSp { addr: SPSAVE });
    ptr_init(asm, WV2_OUT, acc);
    emit_v2_bias_load(asm);
    ld16(asm, Reg16Data::SP, act);
    asm.jp(None, "v2_dispatch");

    // wide entry: same, plus zero the i24 row accumulator.
    asm.label("matvec_v2w");
    asm.i(Instr::LdDirectFromSp { addr: SPSAVE });
    ptr_init(asm, WV2_OUT, acc);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, WV2_ACC);
    a_to(asm, WV2_ACC + 1);
    a_to(asm, WV2_ACC + 2);
    emit_v2_bias_load(asm);
    ld16(asm, Reg16Data::SP, act);
    asm.jp(None, "v2_dispatch");

    // --- dispatch tail ------------------------------------------------------
    // Fetch the next stream byte. A base-81 index (< 81) falls through to the
    // shared computed apply; a sentinel (>= 81) vectors through the 5-entry
    // sentinel table. Decoding trits at runtime (via the `v2_pack` LUT) instead
    // of 81 unrolled handlers keeps the bank-0 routine ~1.5 KiB smaller, which
    // the dense d192 driver needs; the extra cycles are affordable under the
    // 120 s/char budget.
    asm.label("v2_dispatch");
    emit_v2_fetch(asm);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(81),
    });
    asm.jp(Some(Cond::NC), "v2_sentinel");

    // --- computed apply (A = base-81 index, 0..80) --------------------------
    // HL := v2_pack + A; load the 2-bit-packed trits; apply the four columns.
    asm.ld16_label(Reg16Data::HL, "v2_pack", 0);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(asm, WV2_PK);
    asm.i(Instr::Pop {
        dst: Reg16Stack::HL,
    });
    emit_v2_apply_col(asm, Reg8::L, 0);
    emit_v2_apply_col(asm, Reg8::H, 1);
    asm.i(Instr::Pop {
        dst: Reg16Stack::HL,
    });
    emit_v2_apply_col(asm, Reg8::L, 2);
    emit_v2_apply_col(asm, Reg8::H, 3);
    asm.jp(None, "v2_dispatch");

    // --- sentinel vector (A = sentinel byte, 81..85) ------------------------
    asm.label("v2_sentinel");
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(81),
    });
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.ld16_label(Reg16Data::HL, "v2_sent_table", 0);
    asm.i(Instr::AddA {
        src: AluSrc8::Reg(Reg8::L),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::H });
    ld_rr(asm, Reg8::L, Reg8::A);
    asm.i(Instr::JpHl);

    // --- i16 sentinels ------------------------------------------------------
    // ROW_END: store DE, advance out ptr by 2, re-seed bias + SP, keep going.
    asm.label("v2_row_end");
    emit_load_hl_from(asm, WV2_OUT);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    emit_store_hl_to(asm, WV2_OUT);
    ld16(asm, Reg16Data::SP, act);
    emit_v2_bias_load(asm);
    asm.jp(None, "v2_dispatch");

    // MATRIX_END: store the final DE, restore SP, return.
    asm.label("v2_matrix_end");
    emit_load_hl_from(asm, WV2_OUT);
    ld_rr(asm, Reg8::A, Reg8::E);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::D);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    emit_v2_restore_sp_ret(asm);

    // --- wide sentinels -----------------------------------------------------
    // SEG_MID: combine this segment, load the next segment bias, keep the SP
    // walking into the next column block (do NOT re-seed SP).
    asm.label("v2_seg_mid");
    emit_v2_wide_combine(asm);
    emit_v2_bias_load(asm);
    asm.jp(None, "v2_dispatch");

    // ROW_END_WIDE: combine, store the i24 accumulator, advance out ptr by 3,
    // zero the accumulator, re-seed SP + next-row bias.
    asm.label("v2_row_end_wide");
    emit_v2_wide_combine(asm);
    emit_v2_store_wacc(asm, true);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, WV2_ACC);
    a_to(asm, WV2_ACC + 1);
    a_to(asm, WV2_ACC + 2);
    ld16(asm, Reg16Data::SP, act);
    emit_v2_bias_load(asm);
    asm.jp(None, "v2_dispatch");

    // MATRIX_END_WIDE: combine, store the final i24 accumulator, return.
    asm.label("v2_matrix_end_wide");
    emit_v2_wide_combine(asm);
    emit_v2_store_wacc(asm, false);
    emit_v2_restore_sp_ret(asm);

    // --- sentinel vector table (5 entries, LE addresses) --------------------
    asm.label("v2_sent_table");
    asm.word_label("v2_row_end"); // 81 (BASE81_ROW_END)
    asm.word_label("v2_matrix_end"); // 82 (BASE81_MATRIX_END)
    asm.word_label("v2_seg_mid"); // 83 (V2_SEG_MID)
    asm.word_label("v2_row_end_wide"); // 84 (V2_ROW_END_WIDE)
    asm.word_label("v2_matrix_end_wide"); // 85 (V2_MATRIX_END_WIDE)
    // Table length pinned: 81 pattern indices + 5 sentinels.
    debug_assert_eq!(V2_TABLE_LEN, 86);
    let _ = (V2_SEG_MID, V2_ROW_END_WIDE, V2_MATRIX_END_WIDE);

    // --- base-81 -> 2-bit trit pack LUT (81 bytes) --------------------------
    // v2_pack[i] packs base81_pattern(i) as 2 bits per column (00 zero, 01 +1,
    // 10 -1), column k in bits [2k+1:2k].
    asm.label("v2_pack");
    let pack: Vec<u8> = (0..=80_u8)
        .map(|index| {
            let mut byte = 0u8;
            for (k, &w) in crate::spec::base81_pattern(index).iter().enumerate() {
                let field = match w {
                    1 => 0b01,
                    -1 => 0b10,
                    _ => 0b00,
                };
                byte |= field << (2 * k);
            }
            byte
        })
        .collect();
    asm.bytes(pack);
}

/// Apply one packed trit column to the accumulator: extract the 2-bit field
/// `k` from `WV2_PK`, then `DE += reg` (field 01), `DE -= reg` (field 10), or
/// nothing (field 00). `reg` holds the popped activation byte.
fn emit_v2_apply_col(asm: &mut ModelAsm, reg: Reg8, k: u8) {
    a_from(asm, WV2_PK);
    for _ in 0..(2 * k) {
        asm.i(Instr::Rrca);
    }
    asm.i(Instr::AndA {
        src: AluSrc8::Imm(0b11),
    });
    // A: 0 -> skip (carry after `sub 1`), 1 -> plus (zero), 2 -> minus.
    asm.i(Instr::SubA {
        src: AluSrc8::Imm(1),
    });
    let done = asm.fresh("v2col_done");
    asm.jr(Some(Cond::C), &done);
    let plus = asm.fresh("v2col_plus");
    asm.jr(Some(Cond::Z), &plus);
    emit_v2_sub_de(asm, reg);
    asm.jr(None, &done);
    asm.label(&plus);
    emit_v2_add_de(asm, reg);
    asm.label(&done);
}

/// Store `WV2_ACC` (3 bytes) through `WV2_OUT`; when `advance`, bump the out
/// pointer by 3 (row continues); otherwise leave it (final store).
fn emit_v2_store_wacc(asm: &mut ModelAsm, advance: bool) {
    emit_load_hl_from(asm, WV2_OUT);
    for k in 0..3u16 {
        a_from(asm, WV2_ACC + k);
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
    }
    if advance {
        emit_store_hl_to(asm, WV2_OUT);
    }
}

/// Emit the per-token forward pass (embedding copy through `argmax_v`),
/// leaving the argmax at [`S_ARGMAX_ADDR`] and the i24 logits at
/// `layout.logits`. The caller owns the surrounding labels and any decode
/// epilogue (`sample_v`) or loop control.
pub(crate) fn emit_state_forward_body(asm: &mut ModelAsm, plan: &StateRomPlan) {
    let l = &plan.layout;
    let t = &l.topology;
    let params_bank = plan.params_bank as u16;
    let scales_at = |off: u16| CHUNK_ENTRY + off;

    a_from(asm, S_INPUT_ADDR);
    asm.call("emb_copy24");

    let mut chunk_iter = plan.per_matvec_chunks.iter();
    let mut next_bank: u16 = 1;
    let mut mv: usize = 0;

    // --- state stage ---
    asm.call("norm24");
    emit_copy16(asm, l.act, l.dump_snorm, t.d_model);
    emit_state_matvec(asm, plan, &mut mv, &mut chunk_iter, &mut next_bank);
    emit_copy16(asm, l.acc, l.dump_inacc, 2 * t.state_slots);
    set_bank(asm, params_bank);
    asm.call("state_update");
    // out projection across the state-table banks
    ptr_init(asm, OPTR, l.sacc);
    for (i, &rows) in plan.state_bank_rows.iter().enumerate() {
        set_bank(asm, (plan.state_bank0 + i) as u16);
        ptr_init(asm, WPTR, CHUNK_ENTRY);
        ld_r_imm(asm, Reg8::A, rows);
        a_to(asm, ROWCNT);
        asm.call("state_out_mv");
    }
    set_bank(asm, params_bank);
    asm.call("state_out_ep");

    // --- FFN blocks (dense conventions on the widened residual) ---
    for block in 0..t.n_blocks {
        // MoE: run the fixed-point router on the RAW pre-norm residual FIRST
        // (reads l.x, writes EXPERT_SEL), then cache the selected expert's
        // dispatch entry. The router picks the expert index ONLY; the FFN math
        // below is byte-identical to the dense path.
        if let Some(moe) = &plan.moe {
            // Map this block's router bank and enter its resident router code at
            // CHUNK_ENTRY (the router routine + fixed-point helpers live in the
            // SWITCHABLE bank, not bank 0 — the paged-vocab bank-0 driver has no
            // room). `moe_setup` runs the router, picks EXPERT_SEL, and caches
            // the selected expert's dispatch entry into RT_DISP.
            set_bank(asm, moe.router_bank[block] as u16);
            asm.i(Instr::Call {
                cond: None,
                addr: CHUNK_ENTRY,
            });
        }

        asm.call("norm24");
        if block == 0 {
            emit_copy16(asm, l.act, l.dump_norm0, t.d_model);
        }
        if plan.moe.is_some() {
            // up matvec dispatched to the selected expert (narrow, matvec_v2).
            asm.call("moe_up");
        } else {
            emit_state_matvec(asm, plan, &mut mv, &mut chunk_iter, &mut next_bank);
        }
        if block == 0 {
            emit_copy16(asm, l.acc, l.dump_upacc0, 2 * t.d_ff);
        }
        // The up epilogue reads scales via `DE` with its scale bank mapped and
        // the GELU LUT in bank 0. MoE: `moe_up` already mapped this block's
        // scale bank and set `DE = up_scale`. Dense: map the params bank and
        // load `DE` here. NOTE: the block-0 `dump_upacc0` copy above clobbers
        // `DE`, so for MoE block 0 reload `DE = up_scale` after the dump.
        if plan.moe.is_none() {
            set_bank(asm, params_bank);
        }
        // ROWCNT2 := d_ff (16-bit)
        ld_r_imm(asm, Reg8::A, (t.d_ff & 0xFF) as u8);
        a_to(asm, ROWCNT2);
        ld_r_imm(asm, Reg8::A, (t.d_ff >> 8) as u8);
        a_to(asm, ROWCNT2 + 1);
        if plan.moe.is_none() {
            ld16(asm, Reg16Data::DE, scales_at(plan.params.blocks[block].0));
        } else if block == 0 {
            emit_load_de(asm, RT_DISP + 4);
        }
        asm.call("up_ep16");
        if block == 0 {
            emit_copy16(asm, l.act, l.dump_gelu0, t.d_ff);
        }
        if plan.moe.is_some() {
            // down matvec dispatched to the selected expert (wide, matvec_v2w);
            // `moe_down` also maps the scale bank + sets DE = down_scale.
            asm.call("moe_down");
        } else {
            emit_state_matvec(asm, plan, &mut mv, &mut chunk_iter, &mut next_bank);
        }
        if block == 0 {
            emit_copy16(
                asm,
                l.acc,
                l.dump_downacc0,
                l.down_acc_row_bytes() * t.d_model,
            );
        }
        // MoE: `moe_down` mapped the scale bank + set `DE = down_scale`; only
        // block 0's `dump_downacc0` clobbers `DE`, so reload it there.
        if plan.moe.is_none() {
            set_bank(asm, params_bank);
            ld16(asm, Reg16Data::DE, scales_at(plan.params.blocks[block].1));
        } else if block == 0 {
            emit_load_de(asm, RT_DISP + 10);
        }
        match l.down_width {
            AccWidth::I16 => asm.call("down_ep24"),
            AccWidth::I24 => asm.call("down_ep24w"),
        }
        if let Some(xdump) = l.xdump {
            emit_copy16(
                asm,
                l.x,
                xdump + (3 * t.d_model * block) as u16,
                3 * t.d_model,
            );
        }
    }

    asm.call("norm24");
    emit_copy16(asm, l.act, l.dump_qdump, t.d_model);

    match t.logit_paging {
        crate::state_model_ref::LogitPaging::SinglePage => {
            // zero the i24 logits, then accumulate the head lane groups
            emit_zero16(asm, l.logits, (3 * t.vocab) as u16);
            for (g, _) in plan.head_groups.iter().enumerate() {
                set_bank(asm, (plan.head_bank0 + g) as u16);
                asm.call(&format!("head_grp_{g}"));
            }
            asm.call("argmax_v");
        }
        crate::state_model_ref::LogitPaging::Paged => {
            emit_state_paged_epilogue(asm, plan);
        }
    }
}

/// Emit the paged head/argmax epilogue (deploy step 2): stream
/// `n_logit_pages` output-pages of `<= LOGIT_PAGE_IDS` ids, folding each page
/// into the running top-1 (`argmax16`) and the running top-k heap. The single
/// 256-byte logit page is reused per page; the full `3 * vocab` logit vector is
/// never materialized. Leaves the argmax id at `paged.argmax16` and the sorted
/// heap ready for `sample_paged`.
fn emit_state_paged_epilogue(asm: &mut ModelAsm, plan: &StateRomPlan) {
    let l = &plan.layout;
    let t = &l.topology;
    let pg = l.paged.expect("paged layout");
    let n_groups = plan.head_groups.len();
    // Seed the running top-1: best_logit := i24 minimum (raw -2^23 =
    // 0x00_00_80 LE), argmax16 := 0, heap_count := 0.
    ld_r_imm(asm, Reg8::A, 0x00);
    a_to(asm, pg.best_logit);
    a_to(asm, pg.best_logit + 1);
    ld_r_imm(asm, Reg8::A, 0x80);
    a_to(asm, pg.best_logit + 2);
    ld_r_imm(asm, Reg8::A, 0);
    a_to(asm, pg.argmax16);
    a_to(asm, pg.argmax16 + 1);
    a_to(asm, pg.heap_count);

    for page in 0..plan.n_logit_pages {
        let base_id = (page * crate::state_model_ref::LOGIT_PAGE_IDS) as u16;
        let page_len = ((page + 1) * crate::state_model_ref::LOGIT_PAGE_IDS)
            .min(t.vocab)
            .saturating_sub(page * crate::state_model_ref::LOGIT_PAGE_IDS);
        // pg_idx := page ; pg_len := page_len ; pg_base := base_id
        ld_r_imm(asm, Reg8::A, page as u8);
        a_to(asm, pg.pg_idx);
        ld_r_imm(asm, Reg8::A, page_len as u8);
        a_to(asm, pg.pg_len);
        ld_r_imm(asm, Reg8::A, (base_id & 0xFF) as u8);
        a_to(asm, pg.pg_base);
        ld_r_imm(asm, Reg8::A, (base_id >> 8) as u8);
        a_to(asm, pg.pg_base + 1);
        // zero this output-page's logits (3 * page_len bytes)
        emit_zero16(asm, l.logits, (3 * page_len) as u16);
        // accumulate the head lane groups for THIS page's banks
        for (g, _) in plan.head_groups.iter().enumerate() {
            let bank = plan.head_bank0 + page * n_groups + g;
            set_bank(asm, bank as u16);
            asm.call(&format!("head_grp_pg_{g}"));
        }
        asm.call("argmax_fold_pg");
        asm.call("heap_offer_pg");
    }
    // finalize + argmax id -> S_ARGMAX_ADDR low byte (charset feedback) and the
    // high byte -> S_SAMPLED_HI_ADDR (the wide-id feedback high byte, shared by
    // the argmax and sampler paths; only one epilogue runs per ROM).
    a_from(asm, pg.argmax16);
    a_to(asm, S_ARGMAX_ADDR);
    a_from(asm, pg.argmax16 + 1);
    a_to(asm, S_SAMPLED_HI_ADDR);
}

/// Emit every shared routine body plus the bank-0 data tables (GELU/y LUTs
/// and — when a sampler config is present — the exp2 LUT and the sampler
/// routines). Per-row scale/decay tables live in the params bank, not here.
pub(crate) fn emit_state_routines_and_tables(
    asm: &mut ModelAsm,
    model: &IntStateLoweredModel,
    plan: &StateRomPlan,
    sampler: Option<&crate::decode::SamplerConfig>,
) {
    let l = &plan.layout;
    let t = &l.topology;
    match plan.lowering {
        WeightLowering::V3 => emit_chunk_run(asm, plan.animate),
        WeightLowering::V2Dispatch => emit_matvec_v2(asm, l),
    }
    emit_copy_bytes(asm);
    let emb_wide = t.logit_paging == crate::state_model_ref::LogitPaging::Paged;
    emit_emb_copy24(asm, l, plan.emb_bank0 as u16, plan.emb_stride, emb_wide);
    emit_mul16x8(asm);
    emit_mul16(asm);
    emit_isqrt48(asm);
    emit_udiv_norm5(asm);
    // The 254-division twin matches the down-epilogue delta carrier: u16
    // quotient (canonical 65535 clamp) on the i16 path, exact u24 quotient
    // on the wide path (state-int-semantics.v2). Selecting one keeps the
    // arm-B (i16) driver byte-identical to the pre-v2 emission.
    match l.down_width {
        AccWidth::I16 => emit_udiv254(asm),
        AccWidth::I24 => emit_udiv254w(asm),
    }
    let odd = (t.d_model >> t.d_model.trailing_zeros()) as u16;
    if odd > 1 {
        emit_udiv16_odd(asm, odd);
    }
    emit_norm_quant24(asm, l);
    emit_state_update(
        asm,
        l,
        CHUNK_ENTRY + plan.params.state_in_scales,
        CHUNK_ENTRY + plan.params.decay,
    );
    let state_pad = (plan.state_stride - t.state_slots) as u8;
    emit_state_out_matvec(asm, l, state_pad);
    emit_state_out_epilogue(asm, l, CHUNK_ENTRY + plan.params.state_out_scales);
    emit_resid_add24(asm);
    if plan.moe.is_some() {
        // Bank-0 MoE dispatch routines only. The router routine + fixed-point
        // helpers live in the switchable router banks (see `build_moe_router_bank`).
        emit_moe_bank0_routines(asm);
    }
    emit_up_epilogue16(asm, l);
    match l.down_width {
        AccWidth::I16 => emit_down_epilogue24(asm, l),
        AccWidth::I24 => emit_down_epilogue24_wide(asm, l),
    }
    match t.logit_paging {
        crate::state_model_ref::LogitPaging::SinglePage => {
            for (g, &(lo, hi)) in plan.head_groups.iter().enumerate() {
                emit_head_group(asm, l, g, lo, hi);
            }
            emit_argmax(asm, l);
            if let Some(cfg) = sampler {
                emit_rng_step(asm);
                emit_smp_weight(asm, cfg.scale_q16());
                emit_sample_topk(asm, l, cfg.k());
            }
        }
        crate::state_model_ref::LogitPaging::Paged => {
            for (g, &(lo, hi)) in plan.head_groups.iter().enumerate() {
                emit_head_group_paged(asm, l, g, lo, hi);
            }
            emit_argmax_fold_pg(asm, l);
            let heap_k = paged_heap_k(t);
            emit_heap_offer_pg(asm, l, heap_k);
            if let Some(cfg) = sampler {
                emit_rng_step(asm);
                emit_smp_weight(asm, cfg.scale_q16());
                emit_sample_paged(asm, l);
            }
        }
    }

    // bank-0 data: GELU LUT, y LUT, exp2 LUT
    asm.label("gelu_lut");
    asm.bytes(model.gelu_lut.to_vec());
    asm.label("y_lut");
    let mut y_bytes = Vec::with_capacity(model.y_resid_lut.len() * 2);
    for v in &model.y_resid_lut {
        y_bytes.extend_from_slice(&v.to_le_bytes());
    }
    asm.bytes(y_bytes);
    if sampler.is_some() {
        asm.label("exp_lut");
        asm.bytes(crate::decode::build_exp2_lut().to_vec());
    }

    // The MoE per-block expert dispatch tables are NOT bank-0 data: they are
    // prepended to each block's router bank (`MoePlan::router_bank_data`), read
    // by `moe_setup` while that router bank is mapped. This keeps the bank-0
    // driver small (the d192-scale driver is already near the 16 KiB budget).
}

pub(crate) struct BuiltStateRom {
    pub(crate) rom: Vec<u8>,
    pub(crate) rom_size: RomSize,
    pub(crate) bank_count: u16,
    pub(crate) driver_bytes: usize,
    pub(crate) weight_code_bytes: usize,
    pub(crate) weight_chunk_count: usize,
    pub(crate) table_bytes: usize,
    pub(crate) labels: BTreeMap<String, u16>,
}

/// Assemble the banked ROM image: bank-0 driver, weight chunk banks, the
/// params bank, the state/embedding/head table banks, and any
/// `extra_bank_payloads` (each placed at [`CHUNK_ENTRY`] in the banks after
/// the head banks). Returns the ROM bytes, size, and table byte count.
pub(crate) fn assemble_state_rom(
    title: &str,
    driver: Vec<u8>,
    plan: &StateRomPlan,
    model: &IntStateLoweredModel,
    extra_bank_payloads: &[Vec<u8>],
) -> Result<(Vec<u8>, RomSize, usize), ModelRomError> {
    let t = model.topology;
    // State out-projection weight table banks: row-major i8 rows padded to
    // the power-of-two stride.
    let mut state_tables: Vec<Vec<u8>> = Vec::new();
    let mut row = 0usize;
    for &rows in &plan.state_bank_rows {
        let mut bank = Vec::with_capacity(usize::from(rows) * plan.state_stride);
        for _ in 0..rows {
            for &w in model.state_out.row(row) {
                bank.push(w as u8);
            }
            bank.resize(bank.len() + (plan.state_stride - t.state_slots), 0);
            row += 1;
        }
        state_tables.push(bank);
    }
    // Embedding banks: i24 LE rows padded to the power-of-two stride.
    let emb_rpb = BANK_BYTES / plan.emb_stride;
    let mut emb_tables: Vec<Vec<u8>> = Vec::new();
    for bank_idx in 0..plan.emb_bank_count {
        let lo = bank_idx * emb_rpb;
        let hi = ((bank_idx + 1) * emb_rpb).min(t.vocab);
        let mut bank = Vec::with_capacity((hi - lo) * plan.emb_stride);
        for id in lo..hi {
            let before = bank.len();
            // The full `usize` id lookup (`emb_resid_row_at`) is REQUIRED: `id
            // as u8` would alias ids >= 256 (wide-vocab subword models) to rows
            // 0..255. SinglePage/charset (vocab <= 85) is unaffected (id < 256).
            for &v in model.emb_resid_row_at(id) {
                bank.extend_from_slice(&v.to_le_bytes()[..3]);
            }
            bank.resize(before + plan.emb_stride, 0);
        }
        emb_tables.push(bank);
    }
    // Head banks: 64 lane pages of 256 bytes each. SinglePage lays one bank set
    // (page 0 = all `vocab` ids), byte-identical to before paging existed.
    // Paged lays `n_logit_pages` bank sets in page-major order (page, group):
    // set `pg`'s bank for group g holds head weights for lanes [lo..hi) and the
    // <=85 ids [pg*85 .. pg*85+len), stored at lane-page-local offset local_id.
    // The full `usize` id lookup (`head_i8_row_at`) is REQUIRED: `id as u8`
    // would alias ids >= 256 to rows 0..255.
    const PAGE_IDS: usize = crate::state_model_ref::LOGIT_PAGE_IDS;
    let mut head_tables: Vec<Vec<u8>> = Vec::new();
    for page in 0..plan.n_logit_pages {
        let id_lo = page * PAGE_IDS;
        let id_hi = ((page + 1) * PAGE_IDS).min(t.vocab);
        for &(lo, hi) in &plan.head_groups {
            let mut bank = vec![0u8; (hi - lo) * 256];
            for (p, lane) in (lo..hi).enumerate() {
                for (local, global) in (id_lo..id_hi).enumerate() {
                    bank[p * 256 + local] = model.head_i8_row_at(global)[lane] as u8;
                }
            }
            head_tables.push(bank);
        }
    }
    let table_bytes = plan.params.bytes.len()
        + state_tables.iter().map(Vec::len).sum::<usize>()
        + emb_tables.iter().map(Vec::len).sum::<usize>()
        + head_tables.iter().map(Vec::len).sum::<usize>();

    let rom_size = smallest_rom_size(plan.bank_count)?;
    let mut pairs: Vec<(EncodedSection, PlacedSection)> = Vec::new();
    let mut section_seq: u32 = 0;
    let mut push_section = |pairs: &mut Vec<(EncodedSection, PlacedSection)>,
                            bank: usize,
                            cpu_start: u16,
                            bytes: Vec<u8>| {
        let id = SectionId::new(0xB5A7_0000 + section_seq);
        section_seq += 1;
        let size = u16::try_from(bytes.len()).expect("sections fit one bank");
        let placed = PlacedSection {
            id,
            space: if bank == 0 {
                AddressSpace::Rom0
            } else {
                AddressSpace::RomX
            },
            bank: BankIndex::Rom(bank as u16),
            cpu_start,
            final_size: size,
            estimated_size: size,
            alignment_padding: BTreeMap::new(),
        };
        pairs.push((
            EncodedSection {
                id,
                bytes,
                item_spans: Vec::new(),
            },
            placed,
        ));
    };

    push_section(&mut pairs, 0, ENTRY_POINT, driver);
    let mut bank = 1usize;
    match plan.lowering {
        WeightLowering::V3 => {
            for chunks in &plan.per_matvec_chunks {
                for chunk in chunks {
                    push_section(&mut pairs, bank, CHUNK_ENTRY, chunk.clone());
                    bank += 1;
                }
            }
        }
        WeightLowering::V2Dispatch => {
            for packed in &plan.v2_weight_banks {
                push_section(&mut pairs, bank, CHUNK_ENTRY, packed.clone());
                bank += 1;
            }
        }
    }
    debug_assert_eq!(bank, plan.params_bank);
    push_section(
        &mut pairs,
        plan.params_bank,
        CHUNK_ENTRY,
        plan.params.bytes.clone(),
    );
    for (i, table) in state_tables.into_iter().enumerate() {
        push_section(&mut pairs, plan.state_bank0 + i, CHUNK_ENTRY, table);
    }
    for (i, table) in emb_tables.into_iter().enumerate() {
        push_section(&mut pairs, plan.emb_bank0 + i, CHUNK_ENTRY, table);
    }
    for (i, table) in head_tables.into_iter().enumerate() {
        push_section(&mut pairs, plan.head_bank0 + i, CHUNK_ENTRY, table);
    }
    let mut extras_bank0 = plan.head_bank0 + plan.head_groups.len() * plan.n_logit_pages;
    // MoE data banks: per block one router bank (router CODE + dispatch table +
    // router tables, assembled as a self-contained blob entered at CHUNK_ENTRY)
    // then one scale bank.
    if let Some(moe) = &plan.moe {
        for block in 0..t.n_blocks {
            let router_bank_bytes = build_moe_router_bank(
                &plan.layout,
                moe,
                &moe.disp_data[block],
                &moe.router_params[block],
            )?;
            push_section(
                &mut pairs,
                moe.router_bank[block],
                CHUNK_ENTRY,
                router_bank_bytes,
            );
            push_section(
                &mut pairs,
                moe.scale_bank[block],
                CHUNK_ENTRY,
                moe.scale_bank_data[block].clone(),
            );
        }
        extras_bank0 += t.n_blocks * 2;
    }
    for (k, payload) in extra_bank_payloads.iter().enumerate() {
        push_section(&mut pairs, extras_bank0 + k, CHUNK_ENTRY, payload.clone());
    }

    let layout = LayoutPlan {
        sections: pairs.iter().map(|(_, placed)| placed.clone()).collect(),
        bank_count: rom_size.bank_count(),
        free_bytes_per_bank: BTreeMap::new(),
        reserved_ranges: Vec::new(),
    };
    let mut header = CartridgeHeader::new(title)?;
    header.rom_size = rom_size;
    let rom = assemble_rom(&pairs, &layout, &header)?;
    Ok((rom, rom_size, table_bytes))
}

fn build_state_model_rom(
    model: &IntStateLoweredModel,
    loop_tokens: Option<u16>,
    sampler: Option<&crate::decode::SamplerConfig>,
    lowering: WeightLowering,
) -> Result<BuiltStateRom, ModelRomError> {
    let layout = StateWramLayout::plan(model.topology, model.down_width, false)?;
    let plan = plan_state_rom_with(model, layout, 0, lowering)?;
    let l = &plan.layout;

    // Bank-0 driver.
    let mut asm = ModelAsm::new(ENTRY_POINT);
    asm.i(Instr::Di);
    ld16(&mut asm, Reg16Data::SP, S_STACK_TOP);
    if loop_tokens.is_some() {
        asm.i(Instr::XorA {
            src: AluSrc8::Reg(Reg8::A),
        });
        a_to(&mut asm, S_TOKEN_IDX_ADDR);
        a_to(&mut asm, S_DONE_ADDR);
        // The seed id is poked as a u8 at S_INPUT_ADDR; zero its high byte so
        // the first (wide) embedding lookup indexes the seed row exactly.
        a_to(&mut asm, S_INPUT_HI_ADDR);
        // Zero the persistent state once (trained initial-state contract).
        emit_zero16(&mut asm, l.state, (4 * l.topology.state_slots) as u16);
    }
    if sampler.is_some() {
        // Canonicalize the host-poked RNG seed: 0 -> 1 (decode contract).
        a_from(&mut asm, S_RNG_ADDR);
        ld_rr(&mut asm, Reg8::B, Reg8::A);
        a_from(&mut asm, S_RNG_ADDR + 1);
        asm.i(Instr::OrA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::NZ), "rng_seed_ok");
        ld_r_imm(&mut asm, Reg8::A, 1);
        a_to(&mut asm, S_RNG_ADDR);
        asm.label("rng_seed_ok");
    }
    asm.label("token_start");
    emit_state_forward_body(&mut asm, &plan);
    let picked_addr = if sampler.is_some() {
        match model.topology.logit_paging {
            crate::state_model_ref::LogitPaging::SinglePage => asm.call("sample_v"),
            crate::state_model_ref::LogitPaging::Paged => asm.call("sample_paged"),
        }
        S_SAMPLED_ADDR
    } else {
        S_ARGMAX_ADDR
    };
    let wide_feedback = model.topology.logit_paging == crate::state_model_ref::LogitPaging::Paged;
    if let Some(n_tokens) = loop_tokens {
        a_from(&mut asm, S_TOKEN_IDX_ADDR);
        ld_rr(&mut asm, Reg8::L, Reg8::A);
        ld_r_imm(&mut asm, Reg8::H, (l.out >> 8) as u8);
        a_from(&mut asm, picked_addr);
        asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
        a_to(&mut asm, S_INPUT_ADDR);
        if wide_feedback {
            // Wide-vocab subword feedback: carry the picked id's HIGH byte back
            // into S_INPUT_HI so the next embedding lookup indexes the full u16
            // id. The output ring stores the low byte (u8 accessor, host mirror
            // compares the full id via the wide host generator).
            a_from(&mut asm, S_SAMPLED_HI_ADDR);
            a_to(&mut asm, S_INPUT_HI_ADDR);
        }
        a_from(&mut asm, S_TOKEN_IDX_ADDR);
        asm.i(Instr::Inc8 {
            dst: IncDec8Target::Reg(Reg8::A),
        });
        a_to(&mut asm, S_TOKEN_IDX_ADDR);
        asm.i(Instr::CpA {
            src: AluSrc8::Imm((n_tokens & 0xFF) as u8),
        });
        asm.label("token_boundary");
        asm.jp(Some(Cond::NZ), "token_start");
    }
    ld_r_imm(&mut asm, Reg8::A, 1);
    a_to(&mut asm, S_DONE_ADDR);
    asm.label("token_end");
    asm.jr(None, "token_end");

    // routines + bank-0 tables
    emit_state_routines_and_tables(&mut asm, model, &plan, sampler);

    let (driver, labels) = asm.finish()?;
    let driver_bytes = driver.len();
    if usize::from(ENTRY_POINT) + driver_bytes > usize::from(CHUNK_ENTRY) {
        return Err(ModelRomError::DriverOverflowsBank0 {
            bytes: driver_bytes,
        });
    }

    let (rom, rom_size, table_bytes) = assemble_state_rom("GBFSTATE", driver, &plan, model, &[])?;

    Ok(BuiltStateRom {
        rom,
        rom_size,
        bank_count: plan.bank_count as u16,
        driver_bytes,
        weight_code_bytes: plan.weight_code_bytes,
        weight_chunk_count: plan.weight_chunk_count,
        table_bytes,
        labels,
    })
}

/// Assemble the complete stateful one-token ROM. The state vector is NOT
/// initialized by the ROM: the host pokes `layout.state` (and
/// [`S_INPUT_ADDR`]) before running, which lets the gate exercise nonzero
/// carried states.
pub fn build_state_one_token_rom(
    model: &IntStateLoweredModel,
) -> Result<StateOneTokenRom, ModelRomError> {
    build_state_one_token_rom_lowered(model, WeightLowering::V3)
}

/// [`build_state_one_token_rom`] with an explicit weight lowering. V3 is the
/// deployable default; `V2Dispatch` packs the base-81 dispatch streams and is
/// byte-exact against the same host evaluator.
pub fn build_state_one_token_rom_lowered(
    model: &IntStateLoweredModel,
    lowering: WeightLowering,
) -> Result<StateOneTokenRom, ModelRomError> {
    build_state_one_token_rom_debug(model, lowering).map(|(rom, _labels)| rom)
}

/// Same as [`build_state_one_token_rom_lowered`] but also returns the resolved
/// label -> address map. Intended for cycle-profiling and disassembly tooling
/// that needs to attribute an executing PC to a named driver routine; the ROM
/// bytes are identical to the non-debug builder.
pub fn build_state_one_token_rom_debug(
    model: &IntStateLoweredModel,
    lowering: WeightLowering,
) -> Result<(StateOneTokenRom, BTreeMap<String, u16>), ModelRomError> {
    let layout = StateWramLayout::plan(model.topology, model.down_width, false)?;
    let built = build_state_model_rom(model, None, None, lowering)?;
    let labels = built.labels.clone();
    let rom = StateOneTokenRom {
        layout,
        token_start_pc: built.labels["token_start"],
        token_end_pc: built.labels["token_end"],
        rom: built.rom,
        rom_size: built.rom_size,
        bank_count: built.bank_count,
        driver_bytes: built.driver_bytes,
        weight_code_bytes: built.weight_code_bytes,
        weight_chunk_count: built.weight_chunk_count,
        table_bytes: built.table_bytes,
    };
    Ok((rom, labels))
}

/// Assemble the stateful multi-token generation ROM: zeroes the WRAM state
/// once (trained initial-state contract), then generates `n_tokens` steps
/// on-device, feeding each argmax id back and letting the state evolve in
/// WRAM across the whole run.
pub fn build_state_multi_token_rom(
    model: &IntStateLoweredModel,
    n_tokens: u16,
) -> Result<StateMultiTokenRom, ModelRomError> {
    build_state_multi_token_rom_lowered(model, n_tokens, WeightLowering::V3)
}

/// [`build_state_multi_token_rom`] with an explicit weight lowering.
pub fn build_state_multi_token_rom_lowered(
    model: &IntStateLoweredModel,
    n_tokens: u16,
    lowering: WeightLowering,
) -> Result<StateMultiTokenRom, ModelRomError> {
    if n_tokens == 0 || n_tokens > 256 {
        return Err(ModelRomError::BadTokenCount { n_tokens });
    }
    let layout = StateWramLayout::plan(model.topology, model.down_width, false)?;
    let built = build_state_model_rom(model, Some(n_tokens), None, lowering)?;
    Ok(StateMultiTokenRom {
        layout,
        token_start_pc: built.labels["token_start"],
        token_boundary_pc: built.labels["token_boundary"],
        token_end_pc: built.labels["token_end"],
        n_tokens,
        rom: built.rom,
        rom_size: built.rom_size,
        bank_count: built.bank_count,
        driver_bytes: built.driver_bytes,
        weight_code_bytes: built.weight_code_bytes,
        weight_chunk_count: built.weight_chunk_count,
        table_bytes: built.table_bytes,
    })
}

/// Assemble the stateful multi-token **sampling** generation ROM: identical
/// forward pass and WRAM layout to [`build_state_multi_token_rom`] (argmax
/// is still computed and dumped), but each token is decoded by the integer
/// top-k/temperature sampler pinned in [`crate::decode`] and the sampled id
/// is fed back. The host must poke a nonzero XorShift16 seed at
/// [`S_RNG_ADDR`] (2 bytes LE) before running; seed 0 is canonicalized to 1
/// on entry. The output ring receives the sampled ids, which must be
/// byte-identical to `decode::sample_topk` driven by the host integer
/// evaluator with the same seed.
pub fn build_state_multi_token_sampling_rom(
    model: &IntStateLoweredModel,
    n_tokens: u16,
    sampler: &crate::decode::SamplerConfig,
) -> Result<StateMultiTokenRom, ModelRomError> {
    build_state_multi_token_sampling_rom_lowered(model, n_tokens, sampler, WeightLowering::V3)
}

/// [`build_state_multi_token_sampling_rom`] with an explicit weight lowering.
/// The sampler routines add bank-0 code on top of the multi-token driver, so
/// this is where V2's shared-handler footprint is stressed hardest.
pub fn build_state_multi_token_sampling_rom_lowered(
    model: &IntStateLoweredModel,
    n_tokens: u16,
    sampler: &crate::decode::SamplerConfig,
    lowering: WeightLowering,
) -> Result<StateMultiTokenRom, ModelRomError> {
    if n_tokens == 0 || n_tokens > 256 {
        return Err(ModelRomError::BadTokenCount { n_tokens });
    }
    let layout = StateWramLayout::plan(model.topology, model.down_width, false)?;
    let built = build_state_model_rom(model, Some(n_tokens), Some(sampler), lowering)?;
    Ok(StateMultiTokenRom {
        layout,
        token_start_pc: built.labels["token_start"],
        token_boundary_pc: built.labels["token_boundary"],
        token_end_pc: built.labels["token_end"],
        n_tokens,
        rom: built.rom,
        rom_size: built.rom_size,
        bank_count: built.bank_count,
        driver_bytes: built.driver_bytes,
        weight_code_bytes: built.weight_code_bytes,
        weight_chunk_count: built.weight_chunk_count,
        table_bytes: built.table_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_model_ref::{
        StateTopology, synthetic_state_checkpoint, synthetic_state_checkpoint_with,
    };

    #[test]
    fn arm_b_layout_keeps_the_full_dump_surface() {
        let l = StateWramLayout::plan(StateTopology::ARM_B, AccWidth::I16, false).expect("plans");
        assert!(l.xdump.is_some(), "arm-B keeps per-block residual dumps");
        assert!(l.sacc_separate);
        assert_ne!(l.absx, l.acc);
        // No allocation overlaps another (fixed anchors included).
        let mut sorted = l.allocations.clone();
        sorted.sort_unstable();
        for pair in sorted.windows(2) {
            assert!(pair[0].1 <= pair[1].0, "overlap: {pair:?}");
        }
    }

    #[test]
    fn d192_layout_fits_with_documented_degradation() {
        let l = StateWramLayout::plan(StateTopology::D192, AccWidth::I24, false).expect("plans");
        assert!(l.xdump.is_none(), "d192 drops per-block residual dumps");
        assert!(!l.sacc_separate, "d192 overlays the out-acc arena");
        assert_eq!(l.absx, l.acc);
        assert!(l.bytes_allocated <= 8192, "budget: {}", l.bytes_allocated);
        // Shell variant must also fit.
        let ls = StateWramLayout::plan(StateTopology::D192, AccWidth::I24, true).expect("plans");
        assert!(ls.shell.is_some());
    }

    #[test]
    fn untouched_regions_cover_the_gaps() {
        let l = StateWramLayout::plan(StateTopology::ARM_B, AccWidth::I16, false).expect("plans");
        let gaps = l.untouched_regions();
        assert!(!gaps.is_empty());
        for (start, end) in gaps {
            assert!(start < end);
            for &(a0, a1) in &l.allocations {
                assert!(
                    end <= a0 || start >= a1,
                    "gap [{start:#x},{end:#x}) overlaps allocation"
                );
            }
        }
    }

    #[test]
    fn state_one_token_rom_builds_from_synthetic_checkpoint() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let rom = build_state_one_token_rom(&lowered).expect("builds");
        assert_eq!(rom.rom.len(), rom.rom_size.bytes());
        assert!(rom.token_end_pc > rom.token_start_pc);
        assert!(rom.weight_chunk_count >= 9, "state in-proj + 8 FFN matvecs");
        assert_eq!(rom.rom[0x0147], 0x19, "MBC5 cartridge type");
    }

    #[test]
    fn state_multi_token_rom_builds_and_rejects_bad_token_counts() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let rom = build_state_multi_token_rom(&lowered, 256).expect("builds");
        assert_eq!(rom.n_tokens, 256);
        assert!(rom.token_start_pc < rom.token_boundary_pc);
        assert!(rom.token_boundary_pc < rom.token_end_pc);
        let one = build_state_one_token_rom(&lowered).expect("builds");
        assert_eq!(rom.weight_chunk_count, one.weight_chunk_count);
        assert_eq!(rom.table_bytes, one.table_bytes);
        assert!(rom.driver_bytes > one.driver_bytes);
        assert!(matches!(
            build_state_multi_token_rom(&lowered, 0),
            Err(ModelRomError::BadTokenCount { n_tokens: 0 })
        ));
        assert!(matches!(
            build_state_multi_token_rom(&lowered, 257),
            Err(ModelRomError::BadTokenCount { n_tokens: 257 })
        ));
    }

    #[test]
    fn state_sampling_rom_builds_and_shares_the_argmax_layout() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let cfg = crate::decode::SamplerConfig::new(8, 2253).expect("valid sampler");
        let rom = build_state_multi_token_sampling_rom(&lowered, 16, &cfg).expect("builds");
        let argmax = build_state_multi_token_rom(&lowered, 16).expect("builds");
        assert_eq!(rom.n_tokens, 16);
        assert_eq!(rom.weight_chunk_count, argmax.weight_chunk_count);
        assert_eq!(rom.table_bytes, argmax.table_bytes);
        assert_eq!(rom.layout, argmax.layout);
        assert!(
            rom.driver_bytes > argmax.driver_bytes + 256,
            "sampling driver must carry the sampler routines and the 256-byte exp LUT \
             ({} vs {})",
            rom.driver_bytes,
            argmax.driver_bytes
        );
        assert!(matches!(
            build_state_multi_token_sampling_rom(&lowered, 0, &cfg),
            Err(ModelRomError::BadTokenCount { n_tokens: 0 })
        ));
    }

    #[test]
    fn d192_one_token_rom_builds_with_wide_down_chunks_and_banked_tables() {
        let ck = synthetic_state_checkpoint_with(StateTopology::D192, 5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        assert_eq!(lowered.down_width, AccWidth::I24);
        let rom = build_state_one_token_rom(&lowered).expect("builds");
        assert_eq!(rom.rom.len(), rom.rom_size.bytes());
        assert!(
            rom.bank_count > 256,
            "d192 weight code needs the 9-bit MBC5 bank space ({} banks)",
            rom.bank_count
        );
        assert!(rom.bank_count <= 512);
        assert!(
            rom.weight_code_bytes > 2 << 20,
            "several MiB of weight code"
        );
        assert_eq!(rom.rom[0x0147], 0x19, "MBC5 cartridge type");
    }

    // -- MoE ROM builder (deploy step 4) ------------------------------------

    #[test]
    fn moe_one_token_rom_builds_from_synthetic_moe_checkpoint() {
        // A small MoE checkpoint (d192 shape, 4 experts, single-page vocab)
        // must plan and assemble under V2 dispatch, with a MoePlan carrying one
        // (up, down) placement per (block, expert) and a router table per block.
        let ck =
            crate::state_model_ref::synthetic_moe_state_checkpoint(StateTopology::D192_MOE_TEST, 7);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        assert!(lowered.topology.is_moe());
        let layout = StateWramLayout::plan(lowered.topology, lowered.down_width, false)
            .expect("layout plans");
        let plan =
            plan_state_rom_with(&lowered, layout, 0, WeightLowering::V2Dispatch).expect("plans");
        let moe = plan.moe.as_ref().expect("MoE plan present");
        assert_eq!(moe.router_bank.len(), lowered.topology.n_blocks);
        assert_eq!(moe.scale_bank.len(), lowered.topology.n_blocks);
        assert_eq!(moe.disp_data.len(), lowered.topology.n_blocks);
        assert_eq!(moe.router_params.len(), lowered.topology.n_blocks);
        for disp in &moe.disp_data {
            // MOE_DISP_ENTRY (14) bytes per expert dispatch entry.
            assert_eq!(disp.len(), lowered.topology.n_experts * 14);
        }
        // Every expert up/down stream got a distinct placement: the dispatch
        // tables encode distinct (matvec bank, stream pointer) pairs.
        let mut seen = std::collections::BTreeSet::new();
        for disp in &moe.disp_data {
            for e in 0..lowered.topology.n_experts {
                let o = e * 14;
                // up (bank, bc) and down (bank, bc)
                seen.insert((disp[o], disp[o + 1], disp[o + 2], disp[o + 3]));
                seen.insert((disp[o + 6], disp[o + 7], disp[o + 8], disp[o + 9]));
            }
        }
        assert_eq!(
            seen.len(),
            lowered.topology.n_blocks * lowered.topology.n_experts * 2,
            "each expert up/down stream is packed once (distinct dispatch entry)"
        );
        // Full ROM assembles.
        let rom = build_state_one_token_rom_lowered(&lowered, WeightLowering::V2Dispatch)
            .expect("builds");
        assert_eq!(rom.rom.len(), rom.rom_size.bytes());
        assert_eq!(rom.rom[0x0147], 0x19, "MBC5 cartridge type");

        // The REAL deployed shape (Paged vocab-1024, 8 experts) must also fit
        // the bank-0 driver budget and the 512-bank ROM budget. The router code
        // lives in the switchable router banks (not bank 0), which is what keeps
        // the paged-vocab driver under the 0x0150..0x4000 window.
        let ckr =
            crate::state_model_ref::synthetic_moe_state_checkpoint(StateTopology::D192_MOE, 7);
        let lr = IntStateLoweredModel::lower(&ckr).expect("lowers");
        let romr = build_state_one_token_rom_lowered(&lr, WeightLowering::V2Dispatch)
            .expect("real-shape MoE ROM builds within the bank-0 driver budget");
        assert!(romr.bank_count <= 512, "real MoE fits 512 banks");
    }

    #[test]
    fn moe_dispatch_selects_one_expert_per_token() {
        // MoE requires V2 dispatch; V3 must be rejected loudly.
        let ck =
            crate::state_model_ref::synthetic_moe_state_checkpoint(StateTopology::D192_MOE_TEST, 9);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let layout = StateWramLayout::plan(lowered.topology, lowered.down_width, false)
            .expect("layout plans");
        assert!(matches!(
            plan_state_rom_with(&lowered, layout, 0, WeightLowering::V3),
            Err(ModelRomError::UnsupportedTopology { .. })
        ));
        // The host router selects exactly one expert per MoE block per token,
        // and the dispatch table has exactly n_experts entries per block.
        let mut state = lowered.zero_state();
        let trace = lowered.forward_at(3, &mut state);
        assert_eq!(
            trace.selected_experts.len(),
            lowered.topology.n_blocks,
            "one expert chosen per block"
        );
        for &e in &trace.selected_experts {
            assert!(e < lowered.topology.n_experts, "expert in range");
        }
    }

    #[test]
    fn dense_path_byte_identical_when_n_experts_is_1() {
        // A topology with n_experts == 1 is dense (is_moe() == false), so the
        // ROM builder never constructs a MoePlan and the emitted bytes are
        // byte-identical to the pure-dense ROM at the same lowering.
        let topo1 = StateTopology {
            n_experts: 1,
            ..StateTopology::D192
        };
        assert!(!topo1.is_moe());
        let ck = synthetic_state_checkpoint_with(topo1, 5);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let layout = StateWramLayout::plan(lowered.topology, lowered.down_width, false)
            .expect("layout plans");
        let plan =
            plan_state_rom_with(&lowered, layout, 0, WeightLowering::V2Dispatch).expect("plans");
        assert!(plan.moe.is_none(), "n_experts == 1 takes the dense path");

        let dense = synthetic_state_checkpoint_with(StateTopology::D192, 5);
        let dense_lowered = IntStateLoweredModel::lower(&dense).expect("lowers");
        let rom_moe1 = build_state_one_token_rom_lowered(&lowered, WeightLowering::V2Dispatch)
            .expect("builds");
        let rom_dense =
            build_state_one_token_rom_lowered(&dense_lowered, WeightLowering::V2Dispatch)
                .expect("builds");
        assert_eq!(
            rom_moe1.rom, rom_dense.rom,
            "n_experts == 1 ROM is byte-identical to the dense ROM"
        );
    }
}
