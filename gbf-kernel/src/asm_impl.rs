//! Bake-off ternary matvec kernel builders emitting whole bank-0 ROMs.
//!
//! Three strategies over identical fixtures (bd-rzq5n):
//!
//! 1. [`build_v1_interpreted`] — generic loop decoding `Ternary2` packed bytes
//!    field-by-field; smallest code, weights stay data.
//! 2. [`build_v2_dispatch`] — threaded dispatch: each packed byte (base-81
//!    index) jumps through an address table to a handler specialized for that
//!    4-weight pattern; weights stay data, handlers are shared code.
//! 3. [`build_v3_weights_as_code`] — straight-line add/sub code generated from
//!    the weights themselves; no weight data, largest code.
//!
//! Shared machine contract: activations are raw `u8` at zero point 128 in WRAM
//! at [`ACTIVATIONS_BASE`]; outputs are `i16` LE at [`OUTPUT_BASE`]; each row's
//! accumulator is seeded with `-128 * sum(row)` so no on-device sign extension
//! is needed. V2 and V3 walk activations with `SP` (`pop`), so the kernels run
//! with interrupts disabled; that is a bake-off simplification and is called
//! out in the report. All code and data live in ROM bank 0 — no MBC, no bank
//! switching; streaming overhead is a separately-noted cost.

use std::collections::BTreeMap;
use std::fmt;

use gbf_asm::encoder::{EncodeError, EncodedSection, encode_instr};
use gbf_asm::isa::{
    AluSrc8, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Instr, Reg8, Reg16Addr, Reg16Data,
};
use gbf_asm::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};
use gbf_asm::rom::{CartridgeHeader, ENTRY_POINT, RomAssemblyError, RomSize, assemble_rom};
use gbf_asm::section::SectionId;

use crate::spec::{BASE81_SYMBOL_COUNT, TernaryWeights, base81_pattern};

/// WRAM base for the copied activation vector.
pub const ACTIVATIONS_BASE: u16 = 0xC000;
/// WRAM base for the `i16` LE output vector.
pub const OUTPUT_BASE: u16 = 0xC100;
/// Initial stack for prologue/spin code (grows down).
pub const STACK_TOP: u16 = 0xCFFF;

/// ROM0 address of the weight stream (V1 packed / V2 base-81 dispatch bytes).
const WEIGHT_STREAM_ADDR: u16 = 0x3000;
/// ROM0 address of the activation fixture copied to WRAM by the prologue.
const ACTIVATION_FIXTURE_ADDR: u16 = 0x3D00;
/// ROM0 address of the V2 handler address table (256-byte aligned).
const DISPATCH_TABLE_ADDR: u16 = 0x3F00;

/// HRAM scratch offsets (from `0xFF00`).
const HRAM_WEIGHT_BYTE: u8 = 0x80;
const HRAM_BYTE_COUNT: u8 = 0x81;
const HRAM_ROW_COUNT: u8 = 0x82;
const HRAM_OUT_LO: u8 = 0x83;
const HRAM_OUT_HI: u8 = 0x84;

const PROGRAM_SECTION_ID: SectionId = SectionId::new(0xBA40);
const WEIGHT_SECTION_ID: SectionId = SectionId::new(0xBA41);
const ACTIVATION_SECTION_ID: SectionId = SectionId::new(0xBA42);
const TABLE_SECTION_ID: SectionId = SectionId::new(0xBA43);

/// A fully assembled bake-off kernel ROM plus the facts the bench needs.
#[derive(Debug, Clone)]
pub struct KernelRom {
    pub rom: Vec<u8>,
    /// PC at which the measured kernel region begins.
    pub kernel_start_pc: u16,
    /// PC at which the measured kernel region ends.
    pub kernel_end_pc: u16,
    /// Bytes of executable program (prologue + kernel).
    pub program_bytes: usize,
    /// Bytes of ROM-resident kernel data (weight streams, dispatch table);
    /// excludes the activation fixture, which models runtime input.
    pub data_bytes: usize,
}

/// V1: generic interpreted loop over `Ternary2` packed weights.
///
/// Weight stream layout per row: `bias (i16 LE) | fan_in/4 packed bytes`.
pub fn build_v1_interpreted(
    weights: &TernaryWeights,
    activations: &[u8],
) -> Result<KernelRom, KernelBuildError> {
    let shape = weights.shape();
    check_activations(shape.fan_in(), activations)?;

    let mut stream = Vec::new();
    let packed = weights.pack_ternary2();
    let bytes_per_row = usize::from(shape.fan_in()) / 4;
    for row in 0..shape.rows() {
        stream.extend_from_slice(&(weights.row_zero_point_bias(row) as u16).to_le_bytes());
        let start = usize::from(row) * bytes_per_row;
        stream.extend_from_slice(&packed[start..start + bytes_per_row]);
    }

    let mut asm = FlatAsm::new(ENTRY_POINT);
    emit_prologue(&mut asm, shape.fan_in(), activations.len());
    emit_out_ptr_init(&mut asm);
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: rows_u8(shape.rows()),
    });
    emit_ldh_store(&mut asm, HRAM_ROW_COUNT);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: WEIGHT_STREAM_ADDR,
    });

    asm.label("kernel_start");
    asm.label("row_loop");
    // Seed DE with the row bias from the stream.
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::BC,
        imm: ACTIVATIONS_BASE,
    });
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: (shape.fan_in() / 4) as u8,
    });
    emit_ldh_store(&mut asm, HRAM_BYTE_COUNT);

    asm.label("byte_loop");
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    emit_ldh_store(&mut asm, HRAM_WEIGHT_BYTE);
    for field in 0..4_u8 {
        let plus = format!("f{field}_plus");
        let minus = format!("f{field}_minus");
        let next = format!("f{field}_next");
        emit_ldh_load(&mut asm, HRAM_WEIGHT_BYTE);
        // Bring bits 2k+1..2k down to 1..0 with the cheaper rotation direction.
        match field {
            1 => {
                asm.instr(Instr::Rrca);
                asm.instr(Instr::Rrca);
            }
            2 => {
                for _ in 0..4 {
                    asm.instr(Instr::Rrca);
                }
            }
            3 => {
                asm.instr(Instr::Rlca);
                asm.instr(Instr::Rlca);
            }
            _ => {}
        }
        asm.instr(Instr::AndA {
            src: AluSrc8::Imm(3),
        });
        asm.instr(Instr::CpA {
            src: AluSrc8::Imm(1),
        });
        asm.jr(Some(Cond::Z), &plus);
        asm.instr(Instr::CpA {
            src: AluSrc8::Imm(2),
        });
        asm.jr(Some(Cond::Z), &minus);
        asm.instr(Instr::Inc16 { dst: Reg16Data::BC });
        asm.jr(None, &next);

        asm.label(&plus);
        asm.instr(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
        emit_acc_add_de(&mut asm);
        asm.instr(Instr::Inc16 { dst: Reg16Data::BC });
        asm.jr(None, &next);

        asm.label(&minus);
        asm.instr(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
        emit_acc_sub_de_from_a(&mut asm);
        asm.instr(Instr::Inc16 { dst: Reg16Data::BC });

        asm.label(&next);
    }
    emit_ldh_load(&mut asm, HRAM_BYTE_COUNT);
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    emit_ldh_store(&mut asm, HRAM_BYTE_COUNT);
    asm.jp(Some(Cond::NZ), "byte_loop");

    // Row epilogue: store DE through the HRAM output pointer.
    asm.instr(Instr::Push {
        src: gbf_asm::isa::Reg16Stack::HL,
    });
    emit_store_de_via_out_ptr(&mut asm);
    asm.instr(Instr::Pop {
        dst: gbf_asm::isa::Reg16Stack::HL,
    });
    emit_ldh_load(&mut asm, HRAM_ROW_COUNT);
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    emit_ldh_store(&mut asm, HRAM_ROW_COUNT);
    asm.jp(Some(Cond::NZ), "row_loop");
    asm.label("kernel_end");
    emit_spin(&mut asm);

    let (program, labels) = asm.finish()?;
    assemble_kernel_rom(
        program,
        &labels,
        vec![
            (WEIGHT_SECTION_ID, WEIGHT_STREAM_ADDR, stream.clone()),
            (
                ACTIVATION_SECTION_ID,
                ACTIVATION_FIXTURE_ADDR,
                activations.to_vec(),
            ),
        ],
        stream.len(),
        "GBFKV1",
    )
}

/// V2: threaded dispatch through a base-81 handler table.
pub fn build_v2_dispatch(
    weights: &TernaryWeights,
    activations: &[u8],
) -> Result<KernelRom, KernelBuildError> {
    let shape = weights.shape();
    check_activations(shape.fan_in(), activations)?;
    let stream = weights.base81_stream();

    let mut asm = FlatAsm::new(ENTRY_POINT);
    emit_prologue(&mut asm, shape.fan_in(), activations.len());
    emit_out_ptr_init(&mut asm);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::BC,
        imm: WEIGHT_STREAM_ADDR,
    });
    emit_load_bias_from_bc(&mut asm);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::SP,
        imm: ACTIVATIONS_BASE,
    });

    asm.label("kernel_start");
    emit_dispatch(&mut asm);
    for index in 0..=80_u8 {
        asm.label(&format!("h{index}"));
        let pattern = base81_pattern(index);
        for pair in 0..2 {
            asm.instr(Instr::Pop {
                dst: gbf_asm::isa::Reg16Stack::HL,
            });
            for (offset, reg) in [(0, Reg8::L), (1, Reg8::H)] {
                match pattern[pair * 2 + offset] {
                    1 => {
                        asm.instr(Instr::Ld8Reg {
                            dst: Reg8::A,
                            src: reg,
                        });
                        emit_acc_add_de(&mut asm);
                    }
                    -1 => {
                        asm.instr(Instr::Ld8Reg {
                            dst: Reg8::A,
                            src: reg,
                        });
                        emit_acc_sub_de_from_a(&mut asm);
                    }
                    _ => {}
                }
            }
        }
        emit_dispatch(&mut asm);
    }

    // Sentinel 81: finish the row, seed the next one, keep threading.
    asm.label("row_end");
    emit_store_de_via_out_ptr(&mut asm);
    emit_load_bias_from_bc(&mut asm);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::SP,
        imm: ACTIVATIONS_BASE,
    });
    emit_dispatch(&mut asm);

    // Sentinel 82: finish the final row and stop.
    asm.label("matrix_end");
    emit_store_de_via_out_ptr(&mut asm);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::SP,
        imm: STACK_TOP,
    });
    asm.jp(None, "kernel_end");
    asm.label("kernel_end");
    emit_spin(&mut asm);

    let (program, labels) = asm.finish()?;

    // Handler table: 81 pattern handlers plus the two sentinel handlers.
    let mut table = Vec::with_capacity(BASE81_SYMBOL_COUNT * 2);
    for index in 0..=80_u8 {
        table.extend_from_slice(&label_addr(&labels, &format!("h{index}"))?.to_le_bytes());
    }
    table.extend_from_slice(&label_addr(&labels, "row_end")?.to_le_bytes());
    table.extend_from_slice(&label_addr(&labels, "matrix_end")?.to_le_bytes());

    let data_bytes = stream.len() + table.len();
    assemble_kernel_rom(
        program,
        &labels,
        vec![
            (WEIGHT_SECTION_ID, WEIGHT_STREAM_ADDR, stream),
            (TABLE_SECTION_ID, DISPATCH_TABLE_ADDR, table),
            (
                ACTIVATION_SECTION_ID,
                ACTIVATION_FIXTURE_ADDR,
                activations.to_vec(),
            ),
        ],
        data_bytes,
        "GBFKV2",
    )
}

/// V3: straight-line weights-as-code with dual accumulators and zero skipping.
pub fn build_v3_weights_as_code(
    weights: &TernaryWeights,
    activations: &[u8],
) -> Result<KernelRom, KernelBuildError> {
    let shape = weights.shape();
    check_activations(shape.fan_in(), activations)?;

    let mut asm = FlatAsm::new(ENTRY_POINT);
    emit_prologue(&mut asm, shape.fan_in(), activations.len());
    asm.label("kernel_start");
    for row in 0..shape.rows() {
        let row_weights = weights.row(row);
        asm.instr(Instr::Ld16Imm {
            dst: Reg16Data::DE,
            imm: weights.row_zero_point_bias(row) as u16,
        });
        asm.instr(Instr::Ld16Imm {
            dst: Reg16Data::BC,
            imm: 0,
        });
        asm.instr(Instr::Ld16Imm {
            dst: Reg16Data::SP,
            imm: ACTIVATIONS_BASE,
        });

        let mut pending_skip: u16 = 0;
        for pair in row_weights.chunks_exact(2) {
            if pair[0] == 0 && pair[1] == 0 {
                pending_skip += 2;
                continue;
            }
            // Coalesced skip of all-zero pairs; single zero pairs are cheaper
            // to pop-and-discard than to `add sp`.
            while pending_skip > 0 {
                let chunk = pending_skip.min(126);
                if chunk == 2 {
                    asm.instr(Instr::Pop {
                        dst: gbf_asm::isa::Reg16Stack::HL,
                    });
                } else {
                    asm.instr(Instr::AddSp { off: chunk as i8 });
                }
                pending_skip -= chunk;
            }
            asm.instr(Instr::Pop {
                dst: gbf_asm::isa::Reg16Stack::HL,
            });
            for (weight, reg) in [(pair[0], Reg8::L), (pair[1], Reg8::H)] {
                match weight {
                    1 => {
                        asm.instr(Instr::Ld8Reg {
                            dst: Reg8::A,
                            src: reg,
                        });
                        emit_acc_add_de(&mut asm);
                    }
                    -1 => {
                        asm.instr(Instr::Ld8Reg {
                            dst: Reg8::A,
                            src: reg,
                        });
                        // Negative weights accumulate into BC; combined as
                        // P - N at row end.
                        emit_acc_add_bc(&mut asm);
                    }
                    _ => {}
                }
            }
        }
        // Trailing zero-pair skips need no flush: SP is re-seeded per row.

        // y = P - N, stored to the row's static output address.
        let out = OUTPUT_BASE + 2 * row;
        asm.instr(Instr::Ld8Reg {
            dst: Reg8::A,
            src: Reg8::E,
        });
        asm.instr(Instr::SubA {
            src: AluSrc8::Reg(Reg8::C),
        });
        asm.instr(Instr::LdDirectFromA { addr: direct(out) });
        asm.instr(Instr::Ld8Reg {
            dst: Reg8::A,
            src: Reg8::D,
        });
        asm.instr(Instr::SbcA {
            src: AluSrc8::Reg(Reg8::B),
        });
        asm.instr(Instr::LdDirectFromA {
            addr: direct(out + 1),
        });
    }
    asm.label("kernel_end");
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::SP,
        imm: STACK_TOP,
    });
    emit_spin(&mut asm);

    let (program, labels) = asm.finish()?;
    assemble_kernel_rom(
        program,
        &labels,
        vec![(
            ACTIVATION_SECTION_ID,
            ACTIVATION_FIXTURE_ADDR,
            activations.to_vec(),
        )],
        0,
        "GBFKV3",
    )
}

/// `DE += A` (unsigned byte with carry into D), branchless.
///
/// After `add e`, A holds `x + e`; `adc d` folds the carry into a copy of D;
/// `sub e` (the new E) leaves exactly `d + carry`.
fn emit_acc_add_de(asm: &mut FlatAsm) {
    asm.instr(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    asm.instr(Instr::SubA {
        src: AluSrc8::Reg(Reg8::E),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
}

/// `BC += A` — same idiom as [`emit_acc_add_de`] on the BC pair.
fn emit_acc_add_bc(asm: &mut FlatAsm) {
    asm.instr(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.instr(Instr::SubA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
}

/// `DE -= A` (unsigned byte with borrow out of D), branchless.
///
/// `cpl` + set-carry + `adc e` computes `e - x` and leaves carry = no-borrow;
/// `adc 0xFF` then yields `d - 1 + carry`.
fn emit_acc_sub_de_from_a(asm: &mut FlatAsm) {
    asm.instr(Instr::Cpl);
    asm.instr(Instr::Scf);
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::E),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Imm(0xFF),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
}

/// Shared prologue: interrupts off, stack up, copy the activation fixture from
/// ROM into WRAM.
fn emit_prologue(asm: &mut FlatAsm, fan_in: u16, fixture_len: usize) {
    debug_assert_eq!(usize::from(fan_in), fixture_len);
    asm.instr(Instr::Di);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::SP,
        imm: STACK_TOP,
    });
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::HL,
        imm: ACTIVATION_FIXTURE_ADDR,
    });
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::DE,
        imm: ACTIVATIONS_BASE,
    });
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::B,
        imm: fan_in as u8,
    });
    asm.label("copy_loop");
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.instr(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.instr(Instr::Inc16 { dst: Reg16Data::DE });
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::B),
    });
    asm.jr(Some(Cond::NZ), "copy_loop");
}

/// Initialize the HRAM output pointer to [`OUTPUT_BASE`].
fn emit_out_ptr_init(asm: &mut FlatAsm) {
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: (OUTPUT_BASE & 0xFF) as u8,
    });
    emit_ldh_store(asm, HRAM_OUT_LO);
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: (OUTPUT_BASE >> 8) as u8,
    });
    emit_ldh_store(asm, HRAM_OUT_HI);
}

/// Store DE as `i16` LE through the HRAM output pointer and advance it.
/// Clobbers HL and A.
fn emit_store_de_via_out_ptr(asm: &mut FlatAsm) {
    emit_ldh_load(asm, HRAM_OUT_LO);
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    emit_ldh_load(asm, HRAM_OUT_HI);
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::H,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::E,
    });
    asm.instr(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    asm.instr(Instr::LdReg16AddrFromA {
        dst: Reg16Addr::Hli,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    emit_ldh_store(asm, HRAM_OUT_LO);
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    emit_ldh_store(asm, HRAM_OUT_HI);
}

/// Seed DE with the next `i16` LE bias from the BC-walked weight stream.
fn emit_load_bias_from_bc(asm: &mut FlatAsm) {
    asm.instr(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.instr(Instr::Inc16 { dst: Reg16Data::BC });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
    asm.instr(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.instr(Instr::Inc16 { dst: Reg16Data::BC });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
}

/// Threaded-dispatch tail: fetch the next stream byte, double it, index the
/// 256-aligned handler table, and jump. 14 M-cycles.
fn emit_dispatch(asm: &mut FlatAsm) {
    asm.instr(Instr::LdAFromReg16Addr { src: Reg16Addr::BC });
    asm.instr(Instr::Inc16 { dst: Reg16Data::BC });
    asm.instr(Instr::AddA {
        src: AluSrc8::Reg(Reg8::A),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::H,
        imm: (DISPATCH_TABLE_ADDR >> 8) as u8,
    });
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::H });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    asm.instr(Instr::JpHl);
}

fn emit_spin(asm: &mut FlatAsm) {
    asm.label("spin");
    asm.jr(None, "spin");
}

fn emit_ldh_store(asm: &mut FlatAsm, offset: u8) {
    asm.instr(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(offset),
    });
}

fn emit_ldh_load(asm: &mut FlatAsm, offset: u8) {
    asm.instr(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(offset),
    });
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("bake-off data addresses stay below high memory")
}

fn rows_u8(rows: u16) -> u8 {
    u8::try_from(rows).expect("shape validation caps rows at 128")
}

fn check_activations(fan_in: u16, activations: &[u8]) -> Result<(), KernelBuildError> {
    if activations.len() != usize::from(fan_in) {
        return Err(KernelBuildError::ActivationCountMismatch {
            expected: fan_in,
            actual: activations.len(),
        });
    }
    Ok(())
}

fn label_addr(labels: &BTreeMap<String, u16>, name: &str) -> Result<u16, KernelBuildError> {
    labels
        .get(name)
        .copied()
        .ok_or_else(|| KernelBuildError::UndefinedLabel { name: name.into() })
}

/// Assemble the program plus fixed-address ROM0 data sections into a 32 KiB
/// no-MBC ROM image.
fn assemble_kernel_rom(
    program: Vec<u8>,
    labels: &BTreeMap<String, u16>,
    data_sections: Vec<(SectionId, u16, Vec<u8>)>,
    data_bytes: usize,
    title: &str,
) -> Result<KernelRom, KernelBuildError> {
    let program_end = usize::from(ENTRY_POINT) + program.len();
    let first_data_addr = data_sections
        .iter()
        .map(|(_, addr, _)| usize::from(*addr))
        .min()
        .unwrap_or(0x4000);
    if program_end > first_data_addr {
        return Err(KernelBuildError::ProgramOverlapsData {
            program_end,
            first_data_addr,
        });
    }

    let kernel_start_pc = label_addr(labels, "kernel_start")?;
    let kernel_end_pc = label_addr(labels, "kernel_end")?;
    let program_bytes = program.len();

    let mut pairs = Vec::with_capacity(1 + data_sections.len());
    pairs.push(placed_rom0_pair(PROGRAM_SECTION_ID, ENTRY_POINT, program)?);
    for (id, addr, bytes) in data_sections {
        if usize::from(addr) + bytes.len() > 0x4000 {
            return Err(KernelBuildError::DataSectionOverflow { addr });
        }
        pairs.push(placed_rom0_pair(id, addr, bytes)?);
    }

    let layout = LayoutPlan {
        sections: pairs.iter().map(|(_, placed)| placed.clone()).collect(),
        bank_count: RomSize::Kib32.bank_count(),
        free_bytes_per_bank: BTreeMap::new(),
        reserved_ranges: Vec::new(),
    };
    let mut header = CartridgeHeader::new(title)?;
    header.rom_size = RomSize::Kib32;
    let rom = assemble_rom(&pairs, &layout, &header)?;
    Ok(KernelRom {
        rom,
        kernel_start_pc,
        kernel_end_pc,
        program_bytes,
        data_bytes,
    })
}

fn placed_rom0_pair(
    id: SectionId,
    cpu_start: u16,
    bytes: Vec<u8>,
) -> Result<(EncodedSection, PlacedSection), KernelBuildError> {
    let size = u16::try_from(bytes.len())
        .map_err(|_| KernelBuildError::DataSectionOverflow { addr: cpu_start })?;
    let encoded = EncodedSection {
        id,
        bytes,
        item_spans: Vec::new(),
    };
    let placed = PlacedSection {
        id,
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start,
        final_size: size,
        estimated_size: size,
        alignment_padding: BTreeMap::new(),
    };
    Ok((encoded, placed))
}

/// Minimal two-pass flat assembler for single-section bake-off programs.
///
/// `gbf-asm`'s symbolic `Builder` + layout + relaxation pipeline is the
/// production path; the bake-off needs deterministic single-bank programs with
/// exact label PCs, which this ~80-line fixed-size assembler provides without
/// pulling `gbf-runtime`'s banking lowering into `gbf-kernel`.
struct FlatAsm {
    start: u16,
    ops: Vec<FlatOp>,
}

enum FlatOp {
    Instr(Instr),
    Label(String),
    Jp { cond: Option<Cond>, label: String },
    Jr { cond: Option<Cond>, label: String },
}

impl FlatAsm {
    fn new(start: u16) -> Self {
        Self {
            start,
            ops: Vec::new(),
        }
    }

    fn instr(&mut self, instr: Instr) {
        self.ops.push(FlatOp::Instr(instr));
    }

    fn label(&mut self, name: &str) {
        self.ops.push(FlatOp::Label(name.to_owned()));
    }

    fn jp(&mut self, cond: Option<Cond>, label: &str) {
        self.ops.push(FlatOp::Jp {
            cond,
            label: label.to_owned(),
        });
    }

    fn jr(&mut self, cond: Option<Cond>, label: &str) {
        self.ops.push(FlatOp::Jr {
            cond,
            label: label.to_owned(),
        });
    }

    fn finish(self) -> Result<(Vec<u8>, BTreeMap<String, u16>), KernelBuildError> {
        let mut labels = BTreeMap::new();
        let mut pc = self.start;
        for op in &self.ops {
            match op {
                FlatOp::Instr(instr) => pc = pc.wrapping_add(u16::from(instr.byte_len())),
                FlatOp::Jp { .. } => pc = pc.wrapping_add(3),
                FlatOp::Jr { .. } => pc = pc.wrapping_add(2),
                FlatOp::Label(name) => {
                    if labels.insert(name.clone(), pc).is_some() {
                        return Err(KernelBuildError::DuplicateLabel { name: name.clone() });
                    }
                }
            }
        }

        let mut bytes = Vec::new();
        let mut pc = self.start;
        for op in &self.ops {
            match op {
                FlatOp::Instr(instr) => {
                    pc = pc.wrapping_add(u16::from(instr.byte_len()));
                    bytes.extend_from_slice(&encode_instr(instr)?);
                }
                FlatOp::Jp { cond, label } => {
                    pc = pc.wrapping_add(3);
                    let addr = labels.get(label).copied().ok_or_else(|| {
                        KernelBuildError::UndefinedLabel {
                            name: label.clone(),
                        }
                    })?;
                    bytes.extend_from_slice(&encode_instr(&Instr::JpAbs { cond: *cond, addr })?);
                }
                FlatOp::Jr { cond, label } => {
                    pc = pc.wrapping_add(2);
                    let addr = labels.get(label).copied().ok_or_else(|| {
                        KernelBuildError::UndefinedLabel {
                            name: label.clone(),
                        }
                    })?;
                    let offset = i32::from(addr) - i32::from(pc);
                    let offset =
                        i8::try_from(offset).map_err(|_| KernelBuildError::JrOutOfRange {
                            label: label.clone(),
                            offset,
                        })?;
                    bytes.extend_from_slice(&encode_instr(&Instr::JrRel {
                        cond: *cond,
                        off: offset,
                    })?);
                }
                FlatOp::Label(_) => {}
            }
        }
        Ok((bytes, labels))
    }
}

#[derive(Debug)]
pub enum KernelBuildError {
    ActivationCountMismatch {
        expected: u16,
        actual: usize,
    },
    DuplicateLabel {
        name: String,
    },
    UndefinedLabel {
        name: String,
    },
    JrOutOfRange {
        label: String,
        offset: i32,
    },
    ProgramOverlapsData {
        program_end: usize,
        first_data_addr: usize,
    },
    DataSectionOverflow {
        addr: u16,
    },
    Encode(EncodeError),
    RomAssembly(RomAssemblyError),
}

impl fmt::Display for KernelBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ActivationCountMismatch { expected, actual } => {
                write!(f, "expected {expected} activation bytes, got {actual}")
            }
            Self::DuplicateLabel { name } => write!(f, "duplicate label {name}"),
            Self::UndefinedLabel { name } => write!(f, "undefined label {name}"),
            Self::JrOutOfRange { label, offset } => {
                write!(f, "jr to {label} out of range ({offset})")
            }
            Self::ProgramOverlapsData {
                program_end,
                first_data_addr,
            } => write!(
                f,
                "program ends at {program_end:#06x}, past first data section {first_data_addr:#06x}"
            ),
            Self::DataSectionOverflow { addr } => {
                write!(f, "data section at {addr:#06x} overflows ROM0")
            }
            Self::Encode(error) => write!(f, "{error}"),
            Self::RomAssembly(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for KernelBuildError {}

impl From<EncodeError> for KernelBuildError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<RomAssemblyError> for KernelBuildError {
    fn from(error: RomAssemblyError) -> Self {
        Self::RomAssembly(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{TernaryMatvecShape, TernaryWeights, deterministic_activations};

    fn fixture() -> (TernaryWeights, Vec<u8>) {
        let shape = TernaryMatvecShape::new(64, 32).expect("valid shape");
        let weights = TernaryWeights::deterministic(shape, 11, 400).expect("valid weights");
        let activations = deterministic_activations(shape.fan_in(), 12);
        (weights, activations)
    }

    #[test]
    fn all_three_builders_assemble_and_mark_kernel_region() {
        let (weights, activations) = fixture();
        for build in [
            build_v1_interpreted,
            build_v2_dispatch,
            build_v3_weights_as_code,
        ] {
            let rom = build(&weights, &activations).expect("builds");
            assert_eq!(rom.rom.len(), 32 * 1024);
            assert!(rom.kernel_start_pc >= ENTRY_POINT);
            assert!(rom.kernel_end_pc > rom.kernel_start_pc);
            assert!(rom.program_bytes > 0);
        }
    }

    #[test]
    fn v2_dispatch_table_lands_at_fixed_address_with_83_entries() {
        let (weights, activations) = fixture();
        let rom = build_v2_dispatch(&weights, &activations).expect("builds");
        let table_start = usize::from(DISPATCH_TABLE_ADDR);
        let table = &rom.rom[table_start..table_start + BASE81_SYMBOL_COUNT * 2];
        // Every entry must point inside the program region.
        for entry in table.chunks_exact(2) {
            let addr = u16::from_le_bytes([entry[0], entry[1]]);
            assert!(
                addr >= ENTRY_POINT && addr < WEIGHT_STREAM_ADDR,
                "{addr:#06x}"
            );
        }
    }

    #[test]
    fn v3_emits_no_weight_data() {
        let (weights, activations) = fixture();
        let rom = build_v3_weights_as_code(&weights, &activations).expect("builds");
        assert_eq!(rom.data_bytes, 0);
    }
}
