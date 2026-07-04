//! One-token and multi-token forward-pass ROM builders for the LinearState
//! stateful bring-up (stateful deployment of the bd-29ai4 arm-B checkpoint).
//!
//! Builds a banked MBC5 ROM that computes the canonical integer semantics of
//! [`crate::state_model_ref::IntStateLoweredModel::forward`] — including the
//! exact integer recurrence — for one charset id poked into WRAM by the
//! host, and leaves every semantic checkpoint (state-block norm/acc/state/y
//! dumps, per-block residuals, final norm output, i24 logits, argmax, and
//! the persistent state vector itself) in WRAM for byte-exact comparison
//! against the host evaluator.
//!
//! The **state vector lives in WRAM at [`S_STATE_BASE`]** (64 x i32 LE,
//! two's complement) and persists across tokens: the one-token ROM never
//! initializes it (the host pokes it, which lets the gate exercise nonzero
//! carried states), while the multi-token ROM zeroes it once before the
//! generation loop (the trained initial-state contract) and then lets it
//! evolve on-device for the whole run.
//!
//! Differences from the dense builder ([`crate::asm_impl_model`]), all
//! mirroring the canonical integer semantics:
//! - the residual stream is **i24 Q19.5** (3 bytes/lane): norm runs a 7-byte
//!   sum-of-squares, a 48-bit floor isqrt, and 5-byte rounded divisions;
//!   residual adds are 3-byte;
//! - a state stage runs between the embedding and the FFN stack: dense-style
//!   banked in-projection matvec (u8 activations, i16 accumulators), the
//!   per-slot Q8.8 decay multiply-accumulate with i24 saturation, a
//!   loop-driven out-projection matvec over the i32 state (ternary weight
//!   table in its own bank), and the y epilogue (Q8.8 scale, activation
//!   grid, i16 residual-LUT add);
//! - vocabulary is charset_v1's 80 ids (one embedding bank at a 192-byte row
//!   stride, an 80-entry tied head, 240-byte i24 logits).
//!
//! Bank layout: bank 0 driver + integer routines + LUT/scale/decay tables;
//! banks 1..=N V3-style weights-as-code matvec chunks (state in-projection
//! first, then the 8 FFN matvecs in execution order); then the state
//! out-projection weight-table bank, the embedding bank, and the head bank.
//!
//! Decode: argmax is the default (planv0 v0 decode pin). The **sampling
//! variant** ([`build_state_multi_token_sampling_rom`]) additionally emits
//! the integer top-k/temperature sampler pinned in [`crate::decode`]
//! (exp2 LUT in bank 0, XorShift16 state at [`S_RNG_ADDR`], candidate
//! tables at [`S_SAMP_USED`]/[`S_SAMP_IDS`]/[`S_SAMP_WTS`]) and feeds the
//! sampled id back instead of the argmax; argmax is still computed and
//! dumped for debugging.

use std::collections::BTreeMap;

use gbf_asm::encoder::EncodedSection;
use gbf_asm::isa::{
    AluSrc8, BitIndex, CbTarget, Cond, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data,
};
use gbf_asm::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};
use gbf_asm::rom::{CartridgeHeader, ENTRY_POINT, RomSize, assemble_rom};
use gbf_asm::section::SectionId;

use crate::asm_impl_model::{
    ACT_BASE, CHUNK_ENTRY, DIV_NUM, IPTR, LANE, MBC5_ROMB0, MODEL_STACK_TOP, ModelAsm,
    ModelRomError, OPTR, PTR, ROWCNT, SIGN, SPTR, XPTR, a_from, a_to, abs_de_store_sign,
    build_matvec_chunks, emit_copy_bytes, emit_copy_call, emit_mul16, emit_mul16x8, emit_udiv254,
    emit_up_epilogue, ld_r_imm, ld_rr, ld16, load_de_via_ptr, mem_add, mem_copy, mem_shl1,
    mem_shr1, mem_sub_into, smallest_rom_size, zero_mem,
};
use crate::model_ref::{D_FF, D_MODEL};
use crate::state_model_ref::{IntStateLoweredModel, STATE_SLOTS, STATE_VOCAB};

// ---------------------------------------------------------------------------
// WRAM map (all state-ROM addresses; the runner reads these)
// ---------------------------------------------------------------------------

/// Matvec input activations, `u8` zero point 128 (dense address, shared with
/// the reused chunk codegen and epilogues).
pub const S_ACT_BASE: u16 = ACT_BASE;
/// |x| buffer for the widened norm (u24 LE x 64 = 192 bytes).
pub const S_ABSX_BASE: u16 = 0xD300;
/// Matvec raw accumulator outputs (i16 LE, up to 128 rows; dense address).
pub const S_ACC_BASE: u16 = 0xC100;
/// Residual vector x (i24 LE x 64 = 192 bytes).
pub const S_X_BASE: u16 = 0xC300;
/// Persistent recurrent state (i32 LE two's complement x 64 = 256 bytes).
pub const S_STATE_BASE: u16 = 0xC500;
/// State out-projection raw accumulators (i32 LE x 64 = 256 bytes).
pub const S_SACC_BASE: u16 = 0xC600;
/// Head per-lane product LUT pages (lo / hi / sign-extension; dense address).
pub const S_LUT_LO_PAGE: u16 = 0xC700;
/// i24 LE logits x 80 (240 bytes).
pub const S_LOGITS_BASE: u16 = 0xCA00;
/// Per-block residual dumps (4 x 192 bytes).
pub const S_XDUMP_BASE: u16 = 0xCB00;
/// State-block norm output dump (u8 zp128 x 64).
pub const S_DUMP_SNORM: u16 = 0xCE00;
/// State-block y activation dump (u8 zp128 x 64).
pub const S_DUMP_YACT: u16 = 0xCE40;
/// Block-0 norm output dump.
pub const S_DUMP_NORM0: u16 = 0xCE80;
/// Final norm output dump (u8 zp128 x 64).
pub const S_QDUMP_BASE: u16 = 0xCEC0;
/// Block-0 GELU activation dump (128 bytes).
pub const S_DUMP_GELU0: u16 = 0xCF00;
/// Block-0 down accumulator dump (i16 LE x 64).
pub const S_DUMP_DOWNACC0: u16 = 0xCF80;
/// Block-0 up accumulator dump (i16 LE x 128).
pub const S_DUMP_UPACC0: u16 = 0xD000;
/// Argmax id.
pub const S_ARGMAX_ADDR: u16 = 0xD100;
/// Done flag (1 when the run is complete).
pub const S_DONE_ADDR: u16 = 0xD101;
/// Multi-token loop counter.
pub const S_TOKEN_IDX_ADDR: u16 = 0xD102;
/// Sampled id for the current token (sampling variant only; argmax is
/// still computed and dumped at [`S_ARGMAX_ADDR`]).
pub const S_SAMPLED_ADDR: u16 = 0xD103;
/// XorShift16 RNG state (2 bytes LE; the host pokes the seed before
/// running the sampling variant; seed 0 is canonicalized to 1 on entry,
/// mirroring `gbf_kernel::decode::XorShift16::new`).
pub const S_RNG_ADDR: u16 = 0xD104;
/// Input context id; the host pokes this before running.
pub const S_INPUT_ADDR: u16 = 0xD110;
/// Top-k used flags, one byte per charset id (80 bytes; sampling variant).
pub const S_SAMP_USED: u16 = 0xD120;
/// Selected candidate ids in selection order (up to 8; sampling variant).
pub const S_SAMP_IDS: u16 = 0xD170;
/// Selected candidate exp-LUT weights, same order (sampling variant).
pub const S_SAMP_WTS: u16 = 0xD178;
/// State in-projection accumulator dump (i16 LE x 64).
pub const S_DUMP_INACC: u16 = 0xD180;
/// Multi-token output ring (page-aligned, max 256 tokens).
pub const S_OUT_BASE: u16 = 0xD200;
/// Stack top (grows down; well above every buffer).
pub const S_STACK_TOP: u16 = MODEL_STACK_TOP;

// scratch page B (0xC4xx; page A 0xC28x..0xC2Dx is shared with the dense
// routines reused here)
const NORM_SS7: u16 = 0xC400; // 7 bytes sum of squares
const ISQ_IN6: u16 = 0xC408; // 6 bytes
const ISQ_REM6: u16 = 0xC410; // 6 bytes
const ISQ_T16: u16 = 0xC418; // 6 bytes
const ISQ_ROOT6: u16 = 0xC420; // 6 bytes
const NORM_R3: u16 = 0xC428; // 3 bytes (rms raw)
const NORM_D5: u16 = 0xC430; // 5 bytes (8r)
const NORM_D25: u16 = 0xC438; // 5 bytes (16r)
const DIV5_NUM: u16 = 0xC440; // 5 bytes
const DIV5_T1: u16 = 0xC448; // 5 bytes
const DIV5_T2: u16 = 0xC450; // 5 bytes
const SQ_T: u16 = 0xC458; // 4 bytes squaring temp
const ST_H: u16 = 0xC460; // 4 bytes state temp
const ST_P: u16 = 0xC468; // 5 bytes decay product
const ST_M: u16 = 0xC470; // 4 bytes delta m
const SIGN2: u16 = 0xC478; // 1 byte (state sign)
const HI8: u16 = 0xC479; // 1 byte (norm squaring high byte)
const DPTR: u16 = 0xC47A; // 2 bytes decay table pointer
const HPTR: u16 = 0xC47C; // 2 bytes state pointer
const WPTR: u16 = 0xC47E; // (reserved) weight pointer
const ACC4: u16 = 0xC480; // 4 bytes out-matvec accumulator
const OEP_A: u16 = 0xC488; // 5 bytes out-epilogue product
const XP2: u16 = 0xC490; // 2 bytes secondary pointer
const CNT2: u16 = 0xC492; // 1 byte inner counter
const YPTR: u16 = 0xC494; // 2 bytes y dump pointer
const SC2: u16 = 0xC496; // 2 bytes out-epilogue scale

// sampling-decode scratch (0xC4A0..0xC4C2; sampling variant only)
const SMP_M: u16 = 0xC4A0; // 3 bytes max logit (hi byte sign-flipped)
const SMP_BEST: u16 = 0xC4A4; // 3 bytes pass best (hi byte sign-flipped)
const SMP_BESTID: u16 = 0xC4A7; // 1 byte
const SMP_D: u16 = 0xC4A8; // 3 bytes d = max - best (u24)
const SMP_P: u16 = 0xC4B0; // 5 bytes d * scale product
const SMP_TOT: u16 = 0xC4B8; // 2 bytes weight total
const SMP_THR: u16 = 0xC4BA; // 2 bytes draw threshold
const SMP_CUM: u16 = 0xC4BC; // 2 bytes cumulative weight
const SMP_PASS: u16 = 0xC4BE; // 1 byte pass counter
const SMP_RT: u16 = 0xC4C0; // 2 bytes rng shift temp

/// A fully assembled stateful one-token ROM plus the facts the runner needs.
#[derive(Debug, Clone)]
pub struct StateOneTokenRom {
    pub rom: Vec<u8>,
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

/// `ptr` variable += `k` (16-bit).
fn ptr_advance(asm: &mut ModelAsm, ptr: u16, k: u8) {
    a_from(asm, ptr);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(k),
    });
    a_to(asm, ptr);
    a_from(asm, ptr + 1);
    asm.i(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    a_to(asm, ptr + 1);
}

/// Initialize a pointer variable with an immediate address.
fn ptr_init(asm: &mut ModelAsm, ptr: u16, value: u16) {
    ld_r_imm(asm, Reg8::A, (value & 0xFF) as u8);
    a_to(asm, ptr);
    ld_r_imm(asm, Reg8::A, (value >> 8) as u8);
    a_to(asm, ptr + 1);
}

/// Initialize a pointer variable with a label address.
fn ptr_init_label(asm: &mut ModelAsm, ptr: u16, label: &str) {
    asm.ld16_label(Reg16Data::HL, label, 0);
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, ptr);
    ld_rr(asm, Reg8::A, Reg8::H);
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

/// `norm24`: X (i24 x 64 at [`S_X_BASE`]) -> ACT (u8 zp128 x 64), the
/// canonical integer norm+activation-quant on the widened residual.
fn emit_norm_quant24(asm: &mut ModelAsm) {
    asm.label("norm24");
    zero_mem(asm, NORM_SS7, 7);
    // pass 1: abs (u24) + square accumulate into the 7-byte sum
    ld_r_imm(asm, Reg8::A, 64);
    a_to(asm, LANE);
    ptr_init(asm, PTR, S_X_BASE);
    ptr_init(asm, XP2, S_ABSX_BASE);
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

    // mean = SS >> 6 (low 6 bytes), ISQ_IN6 = mean + 1
    for _ in 0..6 {
        mem_shr1(asm, NORM_SS7, 7);
    }
    a_from(asm, NORM_SS7);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    a_to(asm, ISQ_IN6);
    for k in 1..6u16 {
        a_from(asm, NORM_SS7 + k);
        asm.i(Instr::AdcA {
            src: AluSrc8::Imm(0),
        });
        a_to(asm, ISQ_IN6 + k);
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
    ld_r_imm(asm, Reg8::A, 64);
    a_to(asm, LANE);
    ptr_init(asm, PTR, S_X_BASE);
    ptr_init(asm, XP2, S_ABSX_BASE);
    ptr_init(asm, OPTR, S_ACT_BASE);
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
/// the bank-0 decay/scale tables, and updates STATE in place.
fn emit_state_update(asm: &mut ModelAsm) {
    asm.label("state_update");
    ld_r_imm(asm, Reg8::A, STATE_SLOTS as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, HPTR, S_STATE_BASE);
    ptr_init(asm, IPTR, S_ACC_BASE);
    ptr_init_label(asm, SPTR, "scales_state_in");
    ptr_init_label(asm, DPTR, "decay_tab");
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

/// `state_out_mv`: with the state-table bank mapped, accumulate the ternary
/// out-projection over the i32 state into SACC (i32 x 64). The weight table
/// at [`CHUNK_ENTRY`] is row-major `[D_MODEL][STATE_SLOTS]` i8.
fn emit_state_out_matvec(asm: &mut ModelAsm) {
    asm.label("state_out_mv");
    ld_r_imm(asm, Reg8::A, D_MODEL as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, WPTR, CHUNK_ENTRY);
    ptr_init(asm, OPTR, S_SACC_BASE);
    asm.label("smv_row");
    zero_mem(asm, ACC4, 4);
    ptr_init(asm, XP2, S_STATE_BASE);
    ld_r_imm(asm, Reg8::A, STATE_SLOTS as u8);
    a_to(asm, CNT2);
    asm.label("smv_col");
    // w = *WPTR++
    a_from(asm, WPTR);
    ld_rr(asm, Reg8::L, Reg8::A);
    a_from(asm, WPTR + 1);
    ld_rr(asm, Reg8::H, Reg8::A);
    asm.i(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_rr(asm, Reg8::A, Reg8::L);
    a_to(asm, WPTR);
    ld_rr(asm, Reg8::A, Reg8::H);
    a_to(asm, WPTR + 1);
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jp(Some(Cond::Z), "smv_skip");
    // load h (4 bytes via XP2, advancing)
    asm.i(Instr::Push {
        src: gbf_asm::isa::Reg16Stack::BC,
    });
    load_via_ptr_to(asm, XP2, ST_H, 4);
    asm.i(Instr::Pop {
        dst: gbf_asm::isa::Reg16Stack::BC,
    });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(1),
    });
    asm.jr(Some(Cond::NZ), "smv_sub");
    mem_add(asm, ACC4, ST_H, 4);
    asm.jp(None, "smv_next");
    asm.label("smv_sub");
    mem_sub_into(asm, ACC4, ACC4, ST_H, 4);
    asm.jp(None, "smv_next");
    asm.label("smv_skip");
    ptr_advance(asm, XP2, 4);
    asm.label("smv_next");
    a_from(asm, CNT2);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, CNT2);
    asm.jp(Some(Cond::NZ), "smv_col");
    store_via_ptr_from(asm, OPTR, ACC4, 4);
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
/// adds `y_lut[p]` (i16) into the i24 residual.
fn emit_state_out_epilogue(asm: &mut ModelAsm) {
    asm.label("state_out_ep");
    ld_r_imm(asm, Reg8::A, D_MODEL as u8);
    a_to(asm, ROWCNT);
    ptr_init(asm, IPTR, S_SACC_BASE);
    ptr_init(asm, XPTR, S_X_BASE);
    ptr_init(asm, YPTR, S_DUMP_YACT);
    ptr_init_label(asm, SPTR, "scales_state_out");
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
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "sep_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `down_ep24`: DE = scale-table pointer; 64 rows. X[row] (i24) += (mod
/// 2^24) sign(m) * min(65535, (|m|*2 + 127) / 254) with m = scale * acc —
/// the dense Q-grid formula on the widened residual.
fn emit_down_epilogue24(asm: &mut ModelAsm) {
    asm.label("down_ep24");
    ld_rr(asm, Reg8::A, Reg8::E);
    a_to(asm, SPTR);
    ld_rr(asm, Reg8::A, Reg8::D);
    a_to(asm, SPTR + 1);
    ld_r_imm(asm, Reg8::A, 64);
    a_to(asm, ROWCNT);
    ptr_init(asm, IPTR, S_ACC_BASE);
    ptr_init(asm, XPTR, S_X_BASE);
    asm.label("d24_loop");
    load_de_via_ptr(asm, IPTR);
    abs_de_store_sign(asm);
    ld_rr(asm, Reg8::B, Reg8::D);
    ld_rr(asm, Reg8::C, Reg8::E);
    load_de_via_ptr(asm, SPTR);
    asm.call("mul16");
    // DIV_NUM = (MUL_R << 1) + 127
    mem_copy(asm, DIV_NUM, crate::asm_impl_model::MUL_R, 4);
    mem_shl1(asm, DIV_NUM, 4);
    a_from(asm, DIV_NUM);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(127),
    });
    a_to(asm, DIV_NUM);
    carry_ripple(asm, DIV_NUM + 1, 3);
    asm.call("udiv254"); // DE = min(q, 65535)
    // sign-extend to 3 bytes; negate when SIGN
    ld_r_imm(asm, Reg8::B, 0);
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::Z), "d24_add");
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
    asm.label("d24_add");
    // X[row] += B:DE (24-bit wrapping)
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
    a_from(asm, ROWCNT);
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, ROWCNT);
    asm.jp(Some(Cond::NZ), "d24_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `emb_copy24`: A = input id (0..79). Switches to the embedding bank and
/// copies the 192-byte i24 residual row into X.
fn emit_emb_copy24(asm: &mut ModelAsm, emb_bank: u8) {
    asm.label("emb_copy24");
    ld_rr(asm, Reg8::B, Reg8::A);
    ld_r_imm(asm, Reg8::A, emb_bank);
    a_to(asm, MBC5_ROMB0);
    // HL = 0x4000 + id * 192  (id * 192 = (id << 6) + (id << 7))
    ld_rr(asm, Reg8::L, Reg8::B);
    ld_r_imm(asm, Reg8::H, 0);
    for _ in 0..6 {
        asm.i(Instr::AddHl { src: Reg16Data::HL });
    }
    ld_rr(asm, Reg8::D, Reg8::H);
    ld_rr(asm, Reg8::E, Reg8::L);
    asm.i(Instr::AddHl { src: Reg16Data::HL });
    asm.i(Instr::AddHl { src: Reg16Data::DE });
    ld_rr(asm, Reg8::A, Reg8::H);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(asm, Reg8::H, Reg8::A);
    ld16(asm, Reg16Data::DE, S_X_BASE);
    ld_r_imm(asm, Reg8::B, 192);
    asm.jp(None, "copy_bytes");
}

/// `head80`: with the head bank mapped, accumulate all 64 lanes into the 80
/// i24 logits via per-lane product LUTs (dense construction, 80-entry rows).
fn emit_head80(asm: &mut ModelAsm) {
    asm.label("head80");
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.label("h80_lane");
    // q = ACT[lane] - 128
    a_from(asm, LANE);
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, (S_ACT_BASE >> 8) as u8);
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
    asm.jr(Some(Cond::Z), "h80_qpos");
    ld_r_imm(asm, Reg8::B, 0xFF);
    asm.label("h80_qpos");
    // ascending half: entries 0..=127
    ld16(asm, Reg16Data::DE, 0);
    ld16(asm, Reg16Data::HL, S_LUT_LO_PAGE);
    asm.label("h80_asc");
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
    asm.jr(Some(Cond::NZ), "h80_asc");
    // descending half: entries 255 down to 128
    ld16(asm, Reg16Data::DE, 0);
    ld_r_imm(asm, Reg8::L, 0xFF);
    asm.label("h80_desc");
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
    asm.jr(Some(Cond::Z), "h80_desc_done");
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::L),
    });
    asm.jr(None, "h80_desc");
    asm.label("h80_desc_done");
    // sign-extension page
    a_from(asm, SIGN);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.jr(Some(Cond::Z), "h80_sx");
    asm.i(Instr::Cpl);
    asm.label("h80_sx");
    ld_rr(asm, Reg8::C, Reg8::A);
    ld16(asm, Reg16Data::HL, S_LUT_LO_PAGE + 0x200);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.label("h80_fillp");
    ld_rr(asm, Reg8::A, Reg8::B);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(0x80),
    });
    asm.jr(Some(Cond::NZ), "h80_fillp");
    asm.label("h80_filln");
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    ld_rr(asm, Reg8::A, Reg8::L);
    asm.i(Instr::OrA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.jr(Some(Cond::NZ), "h80_filln");
    // accumulate: D = head page (0x40 + lane), E over 0..79, HL = LOGITS
    a_from(asm, LANE);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm((CHUNK_ENTRY >> 8) as u8),
    });
    ld_rr(asm, Reg8::D, Reg8::A);
    ld_r_imm(asm, Reg8::E, 0);
    ld16(asm, Reg16Data::HL, S_LOGITS_BASE);
    asm.label("h80_acc");
    asm.i(Instr::LdAFromReg16Addr { src: Reg16Addr::DE });
    ld_rr(asm, Reg8::C, Reg8::A);
    ld_r_imm(asm, Reg8::B, (S_LUT_LO_PAGE >> 8) as u8);
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
        src: AluSrc8::Imm(STATE_VOCAB as u8),
    });
    asm.jr(Some(Cond::NZ), "h80_acc");
    a_from(asm, LANE);
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    a_to(asm, LANE);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(D_MODEL as u8),
    });
    asm.jp(Some(Cond::NZ), "h80_lane");
    asm.i(Instr::Ret { cond: None });
}

/// `argmax80`: scan the 80 i24 logits, strict-greater update (lowest index
/// wins ties), signed compare via a sign-flipped top byte.
fn emit_argmax80(asm: &mut ModelAsm) {
    use crate::asm_impl_model::{ARG_BEST, ARG_CAND};
    asm.label("argmax80");
    ld16(asm, Reg16Data::HL, S_LOGITS_BASE);
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
    asm.label("a80_loop");
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
        asm.jr(Some(Cond::C), "a80_upd");
        if k > 0 {
            asm.jr(Some(Cond::NZ), "a80_next");
        } else {
            asm.jr(None, "a80_next");
        }
    }
    asm.label("a80_upd");
    mem_copy(asm, ARG_BEST, ARG_CAND, 3);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, S_ARGMAX_ADDR);
    asm.label("a80_next");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(STATE_VOCAB as u8),
    });
    asm.jp(Some(Cond::NZ), "a80_loop");
    asm.i(Instr::Ret { cond: None });
}

/// `rng_step`: advance the XorShift16 state at [`S_RNG_ADDR`] by one step
/// of the pinned (7, 9, 8) triple: `x ^= x << 7; x ^= x >> 9; x ^= x << 8`
/// (byte-serial shifts on the 2-byte LE state; cycle cost is irrelevant
/// next to the matvecs).
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
/// `u = min(255, (d * scale_q16 + 0x8000) >> 16)`, `w = exp_lut[u]` —
/// the same mul16/mul16x8 + round-half-up shape as the deployed
/// state-out epilogue. Mirrors `gbf_kernel::decode` exactly.
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

/// `sample80`: the integer top-k/temperature sampling epilogue. Must run
/// right after `argmax80` (candidate 0 = the argmax with LUT weight
/// `exp_lut[0]`); later passes re-scan the 80 i24 logits skipping used
/// ids (first-unused-then-strictly-greater, so ties go to lower ids),
/// weigh each candidate through `smp_weight`, then draw one XorShift16
/// value, form `threshold = (r * total) >> 16`, and pick the first
/// candidate whose cumulative weight strictly exceeds it. Byte-exact
/// mirror of `gbf_kernel::decode::sample_topk`.
fn emit_sample80(asm: &mut ModelAsm, k: u8) {
    let used_lo = (S_SAMP_USED & 0xFF) as u8;
    let used_hi = (S_SAMP_USED >> 8) as u8;
    let ids_lo = (S_SAMP_IDS & 0xFF) as u8;
    let wts_lo = (S_SAMP_WTS & 0xFF) as u8;
    let logits_hi = (S_LOGITS_BASE >> 8) as u8;
    use crate::asm_impl_model::ARG_CAND;

    asm.label("sample80");
    // clear the 80 used flags
    ld16(asm, Reg16Data::HL, S_SAMP_USED);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_r_imm(asm, Reg8::B, STATE_VOCAB as u8);
    asm.label("s80_clr");
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "s80_clr");

    // candidate 0 = argmax (d = 0 -> u = 0 -> w = exp_lut[0])
    mem_copy(asm, SMP_M, crate::asm_impl_model::ARG_BEST, 3);
    a_from(asm, S_ARGMAX_ADDR);
    a_to(asm, S_SAMP_IDS);
    asm.i(Instr::AddA {
        src: AluSrc8::Imm(used_lo),
    });
    ld_rr(asm, Reg8::L, Reg8::A);
    ld_r_imm(asm, Reg8::H, used_hi);
    ld_r_imm(asm, Reg8::A, 1);
    asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.ld16_label(Reg16Data::HL, "exp_lut", 0);
    asm.i(Instr::Ld8RegFromHl { dst: Reg8::A });
    a_to(asm, S_SAMP_WTS);
    a_to(asm, SMP_TOT);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    a_to(asm, SMP_TOT + 1);
    ld_r_imm(asm, Reg8::A, 1);
    a_to(asm, SMP_PASS);

    // passes 1..k
    asm.label("s80_pass");
    a_from(asm, SMP_PASS);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(k),
    });
    asm.jp(Some(Cond::Z), "s80_draw");
    ld_r_imm(asm, Reg8::A, 0xFF);
    a_to(asm, SMP_BESTID);
    ld_r_imm(asm, Reg8::C, 0);
    asm.label("s80_scan");
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
    asm.jp(Some(Cond::NZ), "s80_next");
    // load logit id C (3 bytes at S_LOGITS_BASE + 3*C), sign-flip hi
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
    asm.jr(Some(Cond::Z), "s80_take");
    // 3-byte unsigned compare: take iff CAND > BEST
    for kb in [2u16, 1, 0] {
        a_from(asm, ARG_CAND + kb);
        ld_rr(asm, Reg8::B, Reg8::A);
        a_from(asm, SMP_BEST + kb);
        asm.i(Instr::CpA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::C), "s80_take");
        if kb > 0 {
            asm.jp(Some(Cond::NZ), "s80_next");
        } else {
            asm.jp(None, "s80_next");
        }
    }
    asm.label("s80_take");
    mem_copy(asm, SMP_BEST, ARG_CAND, 3);
    ld_rr(asm, Reg8::A, Reg8::C);
    a_to(asm, SMP_BESTID);
    asm.label("s80_next");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(STATE_VOCAB as u8),
    });
    asm.jp(Some(Cond::NZ), "s80_scan");
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
    asm.jp(None, "s80_pass");

    // draw: r = rng_step(); threshold = (r * total) >> 16
    asm.label("s80_draw");
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
    asm.label("s80_walk");
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
    asm.jr(Some(Cond::C), "s80_pick");
    asm.i(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    ld_rr(asm, Reg8::A, Reg8::C);
    asm.i(Instr::CpA {
        src: AluSrc8::Imm(k),
    });
    asm.jp(Some(Cond::NZ), "s80_walk");
    // structurally unreachable (threshold < total); defensively pick last
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.label("s80_pick");
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

// ---------------------------------------------------------------------------
// top-level build
// ---------------------------------------------------------------------------

/// Assemble the complete stateful one-token ROM. The state vector is NOT
/// initialized by the ROM: the host pokes [`S_STATE_BASE`] (and
/// [`S_INPUT_ADDR`]) before running, which lets the gate exercise nonzero
/// carried states.
pub fn build_state_one_token_rom(
    model: &IntStateLoweredModel,
) -> Result<StateOneTokenRom, ModelRomError> {
    let built = build_state_model_rom(model, None, None)?;
    Ok(StateOneTokenRom {
        token_start_pc: built.labels["token_start"],
        token_end_pc: built.labels["token_end"],
        rom: built.rom,
        rom_size: built.rom_size,
        bank_count: built.bank_count,
        driver_bytes: built.driver_bytes,
        weight_code_bytes: built.weight_code_bytes,
        weight_chunk_count: built.weight_chunk_count,
        table_bytes: built.table_bytes,
    })
}

/// Assemble the stateful multi-token generation ROM: zeroes the WRAM state
/// once (trained initial-state contract), then generates `n_tokens` steps
/// on-device, feeding each argmax id back and letting the state evolve in
/// WRAM across the whole run.
pub fn build_state_multi_token_rom(
    model: &IntStateLoweredModel,
    n_tokens: u16,
) -> Result<StateMultiTokenRom, ModelRomError> {
    if n_tokens == 0 || n_tokens > 256 {
        return Err(ModelRomError::BadTokenCount { n_tokens });
    }
    let built = build_state_model_rom(model, Some(n_tokens), None)?;
    Ok(StateMultiTokenRom {
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
/// on entry, mirroring `decode::XorShift16::new`. The output ring receives
/// the sampled ids, which must be byte-identical to
/// `decode::sample_topk` driven by the host integer evaluator with the
/// same seed.
pub fn build_state_multi_token_sampling_rom(
    model: &IntStateLoweredModel,
    n_tokens: u16,
    sampler: &crate::decode::SamplerConfig,
) -> Result<StateMultiTokenRom, ModelRomError> {
    if n_tokens == 0 || n_tokens > 256 {
        return Err(ModelRomError::BadTokenCount { n_tokens });
    }
    let built = build_state_model_rom(model, Some(n_tokens), Some(sampler))?;
    Ok(StateMultiTokenRom {
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

/// Weight-chunk plan and bank numbering shared by every stateful ROM
/// variant (one-token, multi-token, sampling, interactive shell).
pub(crate) struct StateRomPlan {
    pub(crate) per_matvec_chunks: Vec<Vec<Vec<u8>>>,
    pub(crate) weight_chunk_count: usize,
    pub(crate) weight_code_bytes: usize,
    pub(crate) state_bank: usize,
    pub(crate) emb_bank: usize,
    pub(crate) head_bank: usize,
    /// Total banks including `extra_banks` appended after the head bank.
    pub(crate) bank_count: usize,
}

/// Build the weight chunks and assign banks: chunks 1..=W, then the state
/// weight-table bank, the embedding bank, the head bank, and finally
/// `extra_banks` variant-owned banks (the shell's UI bank, for example).
pub(crate) fn plan_state_rom(
    model: &IntStateLoweredModel,
    extra_banks: usize,
) -> Result<StateRomPlan, ModelRomError> {
    let mut per_matvec_chunks: Vec<Vec<Vec<u8>>> = Vec::new();
    per_matvec_chunks.push(build_matvec_chunks(&model.state_in)?);
    for (up, down) in &model.blocks {
        per_matvec_chunks.push(build_matvec_chunks(up)?);
        per_matvec_chunks.push(build_matvec_chunks(down)?);
    }
    let weight_chunk_count: usize = per_matvec_chunks.iter().map(Vec::len).sum();
    let weight_code_bytes: usize = per_matvec_chunks
        .iter()
        .flat_map(|chunks| chunks.iter().map(Vec::len))
        .sum();

    let state_bank = 1 + weight_chunk_count;
    let emb_bank = state_bank + 1;
    let head_bank = emb_bank + 1;
    let bank_count = head_bank + 1 + extra_banks;
    if bank_count > 256 {
        // driver bank immediates are u8
        return Err(ModelRomError::TooManyBanks { banks: bank_count });
    }
    Ok(StateRomPlan {
        per_matvec_chunks,
        weight_chunk_count,
        weight_code_bytes,
        state_bank,
        emb_bank,
        head_bank,
        bank_count,
    })
}

/// Emit the per-token forward pass (embedding copy through `argmax80`),
/// leaving the argmax at [`S_ARGMAX_ADDR`] and the i24 logits at
/// [`S_LOGITS_BASE`]. The caller owns the surrounding labels and any decode
/// epilogue (`sample80`) or loop control.
pub(crate) fn emit_state_forward_body(asm: &mut ModelAsm, plan: &StateRomPlan) {
    let state_bank_u8 = plan.state_bank as u8;
    let head_bank_u8 = plan.head_bank as u8;

    a_from(asm, S_INPUT_ADDR);
    asm.call("emb_copy24");

    let mut chunk_iter = plan.per_matvec_chunks.iter();
    let mut next_bank: u8 = 1;
    let call_chunks = |asm: &mut ModelAsm, chunks: &Vec<Vec<u8>>, next_bank: &mut u8| {
        for _ in chunks {
            ld_r_imm(asm, Reg8::A, *next_bank);
            a_to(asm, MBC5_ROMB0);
            asm.i(Instr::Call {
                cond: None,
                addr: CHUNK_ENTRY,
            });
            *next_bank += 1;
        }
    };

    // --- state stage ---
    asm.call("norm24");
    emit_copy_call(asm, S_ACT_BASE, S_DUMP_SNORM, 64);
    let in_chunks = chunk_iter.next().expect("state in-proj chunks exist");
    call_chunks(asm, in_chunks, &mut next_bank);
    emit_copy_call(asm, S_ACC_BASE, S_DUMP_INACC, 128);
    asm.call("state_update");
    ld_r_imm(asm, Reg8::A, state_bank_u8);
    a_to(asm, MBC5_ROMB0);
    asm.call("state_out_mv");
    asm.call("state_out_ep");

    // --- FFN blocks (dense conventions on the widened residual) ---
    for block in 0..crate::model_ref::N_BLOCKS {
        asm.call("norm24");
        if block == 0 {
            emit_copy_call(asm, S_ACT_BASE, S_DUMP_NORM0, 64);
        }
        let up_chunks = chunk_iter.next().expect("up chunks exist");
        call_chunks(asm, up_chunks, &mut next_bank);
        if block == 0 {
            emit_copy_call(asm, S_ACC_BASE, S_DUMP_UPACC0, 0); // 256 bytes
        }
        asm.ld16_label(Reg16Data::DE, &format!("scales_up_{block}"), 0);
        ld_r_imm(asm, Reg8::A, D_FF as u8);
        asm.call("up_epilogue");
        if block == 0 {
            emit_copy_call(asm, S_ACT_BASE, S_DUMP_GELU0, 128);
        }
        let down_chunks = chunk_iter.next().expect("down chunks exist");
        call_chunks(asm, down_chunks, &mut next_bank);
        if block == 0 {
            emit_copy_call(asm, S_ACC_BASE, S_DUMP_DOWNACC0, 128);
        }
        asm.ld16_label(Reg16Data::DE, &format!("scales_down_{block}"), 0);
        asm.call("down_ep24");
        emit_copy_call(asm, S_X_BASE, S_XDUMP_BASE + 192 * block as u16, 192);
    }

    asm.call("norm24");
    emit_copy_call(asm, S_ACT_BASE, S_QDUMP_BASE, 64);

    // zero the 240-byte logits buffer
    ld16(asm, Reg16Data::HL, S_LOGITS_BASE);
    asm.i(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    ld_r_imm(asm, Reg8::B, 240);
    let zl = asm.fresh("zl80");
    asm.label(&zl);
    asm.i(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.i(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), &zl);

    ld_r_imm(asm, Reg8::A, head_bank_u8);
    a_to(asm, MBC5_ROMB0);
    asm.call("head80");
    asm.call("argmax80");
}

/// Emit every shared routine body plus the bank-0 data tables (GELU/y LUTs,
/// state scale/decay tables, per-block scales, and — when a sampler config
/// is present — the exp2 LUT and the sampler routines).
pub(crate) fn emit_state_routines_and_tables(
    asm: &mut ModelAsm,
    model: &IntStateLoweredModel,
    emb_bank_u8: u8,
    sampler: Option<&crate::decode::SamplerConfig>,
) {
    emit_copy_bytes(asm);
    emit_emb_copy24(asm, emb_bank_u8);
    emit_mul16x8(asm);
    emit_mul16(asm);
    emit_isqrt48(asm);
    emit_udiv_norm5(asm);
    emit_udiv254(asm);
    emit_norm_quant24(asm);
    emit_state_update(asm);
    emit_state_out_matvec(asm);
    emit_state_out_epilogue(asm);
    emit_up_epilogue(asm);
    emit_down_epilogue24(asm);
    emit_head80(asm);
    emit_argmax80(asm);
    if let Some(cfg) = sampler {
        emit_rng_step(asm);
        emit_smp_weight(asm, cfg.scale_q16());
        emit_sample80(asm, cfg.k());
    }

    // bank-0 data: GELU LUT, y LUT, state scale/decay tables, block scales
    asm.label("gelu_lut");
    asm.bytes(model.gelu_lut.to_vec());
    asm.label("y_lut");
    let mut y_bytes = Vec::with_capacity(model.y_resid_lut.len() * 2);
    for v in &model.y_resid_lut {
        y_bytes.extend_from_slice(&v.to_le_bytes());
    }
    asm.bytes(y_bytes);
    asm.label("scales_state_in");
    let mut si_bytes = Vec::with_capacity(STATE_SLOTS * 2);
    for row in 0..STATE_SLOTS {
        si_bytes.extend_from_slice(&model.state_in.layer.scale_raw(row).to_le_bytes());
    }
    asm.bytes(si_bytes);
    asm.label("scales_state_out");
    let mut so_bytes = Vec::with_capacity(D_MODEL * 2);
    for row in 0..D_MODEL {
        so_bytes.extend_from_slice(&model.state_out.scale_raw(row).to_le_bytes());
    }
    asm.bytes(so_bytes);
    asm.label("decay_tab");
    asm.bytes(model.decay_u8.clone());
    if sampler.is_some() {
        asm.label("exp_lut");
        asm.bytes(crate::decode::build_exp2_lut().to_vec());
    }
    for (block, (up, down)) in model.blocks.iter().enumerate() {
        asm.label(&format!("scales_up_{block}"));
        let mut up_bytes = Vec::with_capacity(up.layer.rows() * 2);
        for row in 0..up.layer.rows() {
            up_bytes.extend_from_slice(&up.layer.scale_raw(row).to_le_bytes());
        }
        asm.bytes(up_bytes);
        asm.label(&format!("scales_down_{block}"));
        let mut down_bytes = Vec::with_capacity(down.layer.rows() * 2);
        for row in 0..down.layer.rows() {
            down_bytes.extend_from_slice(&down.layer.scale_raw(row).to_le_bytes());
        }
        asm.bytes(down_bytes);
    }
}

/// Assemble the banked ROM image: bank-0 driver, weight chunk banks, the
/// state/embedding/head table banks, and any `extra_bank_payloads` (each
/// placed at [`CHUNK_ENTRY`] in the banks after the head bank). Returns the
/// ROM bytes, size, and the model table byte count.
pub(crate) fn assemble_state_rom(
    title: &str,
    driver: Vec<u8>,
    plan: &StateRomPlan,
    model: &IntStateLoweredModel,
    extra_bank_payloads: &[Vec<u8>],
) -> Result<(Vec<u8>, RomSize, usize), ModelRomError> {
    // Banked data tables.
    // State out-projection weight table: row-major [D_MODEL][STATE_SLOTS] i8.
    let mut state_table = Vec::with_capacity(D_MODEL * STATE_SLOTS);
    for row in 0..D_MODEL {
        for &w in model.state_out.row(row) {
            state_table.push(w as u8);
        }
    }
    // Embedding bank: 192-byte i24 LE rows, 80 ids.
    let mut emb_table = Vec::with_capacity(STATE_VOCAB * D_MODEL * 3);
    for id in 0..STATE_VOCAB {
        for &v in model.emb_resid_row(id as u8) {
            emb_table.extend_from_slice(&v.to_le_bytes()[..3]);
        }
    }
    // Head bank: 64 lane pages of 256 bytes (entries 0..79 valid).
    let mut head_table = vec![0u8; D_MODEL * 256];
    for lane in 0..D_MODEL {
        for id in 0..STATE_VOCAB {
            head_table[lane * 256 + id] = model.head_i8_row(id as u8)[lane] as u8;
        }
    }
    let table_bytes = state_table.len() + emb_table.len() + head_table.len();

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
    for chunks in &plan.per_matvec_chunks {
        for chunk in chunks {
            push_section(&mut pairs, bank, CHUNK_ENTRY, chunk.clone());
            bank += 1;
        }
    }
    debug_assert_eq!(bank, plan.state_bank);
    push_section(&mut pairs, plan.state_bank, CHUNK_ENTRY, state_table);
    push_section(&mut pairs, plan.emb_bank, CHUNK_ENTRY, emb_table);
    push_section(&mut pairs, plan.head_bank, CHUNK_ENTRY, head_table);
    for (k, payload) in extra_bank_payloads.iter().enumerate() {
        push_section(
            &mut pairs,
            plan.head_bank + 1 + k,
            CHUNK_ENTRY,
            payload.clone(),
        );
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
) -> Result<BuiltStateRom, ModelRomError> {
    let plan = plan_state_rom(model, 0)?;
    let emb_bank_u8 = plan.emb_bank as u8;

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
        // Zero the persistent state once (trained initial-state contract).
        ld16(&mut asm, Reg16Data::HL, S_STATE_BASE);
        ld_r_imm(&mut asm, Reg8::B, 0);
        asm.label("zs_loop");
        asm.i(Instr::LdReg16AddrFromA {
            dst: Reg16Addr::Hli,
        });
        asm.i(Instr::Dec8 {
            dst: IncDec8Target::Reg(Reg8::B),
        });
        asm.jr(Some(Cond::NZ), "zs_loop");
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
        asm.call("sample80");
        S_SAMPLED_ADDR
    } else {
        S_ARGMAX_ADDR
    };
    if let Some(n_tokens) = loop_tokens {
        a_from(&mut asm, S_TOKEN_IDX_ADDR);
        ld_rr(&mut asm, Reg8::L, Reg8::A);
        ld_r_imm(&mut asm, Reg8::H, (S_OUT_BASE >> 8) as u8);
        a_from(&mut asm, picked_addr);
        asm.i(Instr::Ld8HlFromReg { src: Reg8::A });
        a_to(&mut asm, S_INPUT_ADDR);
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
    emit_state_routines_and_tables(&mut asm, model, emb_bank_u8, sampler);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state_model_ref::synthetic_state_checkpoint;

    #[test]
    fn state_one_token_rom_builds_from_synthetic_checkpoint() {
        let ck = synthetic_state_checkpoint(11);
        let lowered = IntStateLoweredModel::lower(&ck).expect("lowers");
        let rom = build_state_one_token_rom(&lowered).expect("builds");
        assert_eq!(rom.rom.len(), rom.rom_size.bytes());
        assert!(rom.token_end_pc > rom.token_start_pc);
        assert!(rom.weight_chunk_count >= 9, "state in-proj + 8 FFN matvecs");
        assert_eq!(
            rom.table_bytes,
            D_MODEL * STATE_SLOTS + STATE_VOCAB * D_MODEL * 3 + D_MODEL * 256
        );
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
}
