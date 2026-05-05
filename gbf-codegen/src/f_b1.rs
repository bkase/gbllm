//! F-B1 Compute Bringup request, plans, fixtures, and skeletal lowering.
//!
//! This module intentionally keeps F-B1-specific types named as bringup types.
//! M1's durable `CompileRequest`, full sched IR, and resource validators are
//! separate owner beads.

use std::collections::BTreeSet;
use std::fmt;

use gbf_abi::compute_shape::SquareDim;
use gbf_asm::builder::Builder;
use gbf_asm::effect::MachineEffectKind;
use gbf_asm::encoder::{EncodeError, EncodedSection, encode_instr, encode_section};
use gbf_asm::isa::{
    AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Instr, Reg8,
    Reg16Addr, Reg16Data, Reg16Stack,
};
use gbf_asm::layout::{
    AddressSpace, BankIndex, LayoutPlan, PinnedPlacement, PlacedSection, PlacementProfile,
    ROM0_END_EXCLUSIVE, layout_into_banks,
};
use gbf_asm::lowering::lower_pre_layout_ops;
use gbf_asm::relax::relax_and_legalize;
use gbf_asm::rom::{CartridgeHeader, RomAssemblyError, RomSize, assemble_rom, global_checksum};
use gbf_asm::section::{
    BankLeaseSpec, BankReleaseDisposition, BranchKind, LeaseGeneration, LeaseId, LeaseLifetime,
    MbcBankClass, PreLayoutOp, Section, SectionId, SectionPrivilege, SectionRole, SymbolicBranch,
    YieldKind,
};
use gbf_asm::symbols::{SymbolName, SymbolTable};
use gbf_foundation::{CompileProfileId, Hash256, TargetProfileId};
use gbf_ir::infer::{FusedTileSize, InferGraph, MatmulI8Node, TensorBinding, TensorStorage};
use gbf_runtime::banking::{
    BankingPreLayoutLowering, ReturnRomBank, ReturnState, ValidatedBankLeaseSpec,
    lease_rom_switchable, release_bank,
};
use gbf_runtime::trace::{FB1TraceEvent, YieldQuantumKind, emit_fb1_event};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const BRINGUP_COMPILE_PROFILE_ID: &str = "bringup";
pub const QUARTER_SQUARE_MIN: i16 = -256;
pub const QUARTER_SQUARE_MAX: i16 = 255;
pub const QUARTER_SQUARE_LEN: usize = 512;
pub const QUARTER_SQUARE_BYTES: u16 = 1024;
pub const ROM_BANK_SIZE_BYTES: u32 = gbf_hw::memory::SWITCHABLE_BANK_SIZE_BYTES;
pub const TILE_EDGE: u16 = 16;
pub const ACCUMULATOR_TILE_BYTES: u16 = 16 * 16 * 4;
pub const PANEL_TILE_BYTES: u16 = 16 * 16;
pub const FB1_KERNEL_SECTION_ID: SectionId = SectionId::new(0xB100);
pub const FB1_L0_ROM_SECTION_ID: SectionId = SectionId::new(0xB101);
pub const FB1_VBLANK_HANDLER_SECTION_ID: SectionId = SectionId::new(0xB10A);
pub const FB1_L0_A_BASE: u16 = 0xC000;
pub const FB1_L0_B_BASE: u16 = 0xC100;
pub const FB1_L0_OUTPUT_BASE: u16 = 0xC200;
pub const FB1_L2_A_ROM_BANK: u16 = 1;
pub const FB1_L2_B_ROM_BANK: u16 = 2;
pub const EMITTED_MULTIPLY_KERNEL_ID: &str = "fixed_8_step_shift_add_i8_i32";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeBringupRequest {
    pub target_profile_id: TargetProfileId,
    pub kernel_impl_id: KernelImplId,
    pub compile_profile_id: CompileProfileId,
    pub matrix_dim: SquareDim,
    pub tile_size: TileSize,
    pub operand_fixture: OperandFixtureSpec,
    pub operand_layout: OperandLayout,
    pub yield_quantum: YieldQuantum,
    // M1: real CompileRequest will replace this type entirely; deferred fields
    // are listed in F-B1 §7.3. Do not add quant_policy, calibration_set_ref,
    // risk_policy, objective, repair_policy, observability_mode, trace_budget,
    // or data_lowering_profile_id to this F-B1-local request.
}

impl ComputeBringupRequest {
    #[must_use]
    pub fn l1_wram_smoke() -> Self {
        Self {
            target_profile_id: TargetProfileId::from_static(
                gbf_hw::target::BRINGUP_TARGET_PROFILE_ID,
            ),
            kernel_impl_id: KernelImplId::QuarterSquareTableV1,
            compile_profile_id: CompileProfileId::from_static(BRINGUP_COMPILE_PROFILE_ID),
            matrix_dim: SquareDim::new(16).expect("16 is valid"),
            tile_size: TileSize::f_b1(),
            operand_fixture: OperandFixtureSpec::DeterministicAffineV1,
            operand_layout: OperandLayout::WramSmoke,
            yield_quantum: YieldQuantum::default(),
        }
    }

    #[must_use]
    pub fn headline_n128() -> Self {
        Self {
            matrix_dim: SquareDim::new(128).expect("128 is valid"),
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank 1"),
                    offset: RomBankOffset::ZERO,
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank 2"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..Self::l1_wram_smoke()
        }
    }

    pub fn validate(&self) -> Result<(), ComputeBringupRequestError> {
        if self.compile_profile_id.as_str() != BRINGUP_COMPILE_PROFILE_ID {
            return Err(ComputeBringupRequestError::NonBringupCompileProfile {
                found: self.compile_profile_id.as_str().to_owned(),
            });
        }
        if self.target_profile_id.as_str() != gbf_hw::target::BRINGUP_TARGET_PROFILE_ID {
            return Err(ComputeBringupRequestError::UnsupportedTarget {
                found: self.target_profile_id.as_str().to_owned(),
            });
        }
        if self.kernel_impl_id != KernelImplId::QuarterSquareTableV1 {
            return Err(ComputeBringupRequestError::UnsupportedKernel);
        }
        if self.tile_size != TileSize::f_b1() {
            return Err(ComputeBringupRequestError::BadTile {
                found: self.tile_size,
            });
        }
        if self.operand_fixture != OperandFixtureSpec::DeterministicAffineV1 {
            return Err(ComputeBringupRequestError::UnsupportedFixture);
        }
        self.operand_layout.validate(self.matrix_dim)?;
        Ok(())
    }

    pub fn hash(&self) -> Result<Hash256, ComputeBringupRequestError> {
        self.validate()?;
        let bytes = serde_json::to_vec(self).map_err(|error| ComputeBringupRequestError::Hash {
            reason: error.to_string(),
        })?;
        Ok(hash_domain(b"gbf-codegen/f-b1/request", &bytes))
    }

    pub fn emit_imported_event(&self) -> Result<Hash256, ComputeBringupRequestError> {
        let hash = self.hash()?;
        emit_fb1_event(&FB1TraceEvent::ComputeReqImported {
            request_hash: format!("sha256:{hash}"),
        });
        Ok(hash)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum KernelImplId {
    QuarterSquareTableV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum OperandFixtureSpec {
    DeterministicAffineV1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileSize {
    pub m: u16,
    pub n: u16,
    pub k: u16,
}

impl TileSize {
    #[must_use]
    pub const fn f_b1() -> Self {
        Self {
            m: TILE_EDGE,
            n: TILE_EDGE,
            k: TILE_EDGE,
        }
    }
}

impl From<TileSize> for FusedTileSize {
    fn from(value: TileSize) -> Self {
        Self {
            m: value.m,
            n: value.n,
            k: value.k,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "PascalCase")]
pub enum YieldQuantum {
    KLaneFullTile,
    KLaneRows4,
    #[default]
    KLaneRow,
}

impl YieldQuantum {
    #[must_use]
    pub const fn products_per_quantum(self) -> u16 {
        match self {
            Self::KLaneFullTile => 16 * 16,
            Self::KLaneRows4 => 4 * 16,
            Self::KLaneRow => 16,
        }
    }
}

impl From<YieldQuantum> for YieldQuantumKind {
    fn from(value: YieldQuantum) -> Self {
        match value {
            YieldQuantum::KLaneFullTile => Self::KLaneFullTile,
            YieldQuantum::KLaneRows4 => Self::KLaneRows4,
            YieldQuantum::KLaneRow => Self::KLaneRow,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperandLayout {
    WramSmoke,
    DistinctRomBanks {
        a: BankedOperandPlacement,
        b: BankedOperandPlacement,
    },
}

impl OperandLayout {
    pub fn validate(&self, dim: SquareDim) -> Result<(), ComputeBringupRequestError> {
        match self {
            Self::WramSmoke => {
                if dim.n() == 16 {
                    Ok(())
                } else {
                    Err(ComputeBringupRequestError::SmokeLayoutRequiresN16 { found: dim.n() })
                }
            }
            Self::DistinctRomBanks { a, b } => {
                if a.bank == b.bank {
                    return Err(ComputeBringupRequestError::SameBank { bank: a.bank.get() });
                }
                let len = u32::try_from(dim.operand_bytes()).expect("operand bytes fit u32");
                validate_placement("A", *a, len, dim)?;
                validate_placement("B", *b, len, dim)?;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BankedOperandPlacement {
    pub bank: RomBankId,
    pub offset: RomBankOffset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct RomBankId(u16);

impl RomBankId {
    pub const MAX_MBC5: u16 = 511;

    pub const fn new(bank: u16) -> Result<Self, ComputeBringupRequestError> {
        if bank == 0 {
            return Err(ComputeBringupRequestError::BankZero);
        }
        if bank > Self::MAX_MBC5 {
            return Err(ComputeBringupRequestError::BankOutOfRange { bank });
        }
        Ok(Self(bank))
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for RomBankId {
    type Error = ComputeBringupRequestError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RomBankId> for u16 {
    fn from(value: RomBankId) -> Self {
        value.0
    }
}

impl<'de> Deserialize<'de> for RomBankId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bank = u16::deserialize(deserializer)?;
        Self::new(bank).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(from = "u16", into = "u16")]
pub struct RomBankOffset(u16);

impl RomBankOffset {
    pub const ZERO: Self = Self(0);

    pub const fn new(offset: u16) -> Self {
        Self(offset)
    }

    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }
}

impl From<RomBankOffset> for u16 {
    fn from(value: RomBankOffset) -> Self {
        value.0
    }
}

impl From<u16> for RomBankOffset {
    fn from(value: u16) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ComputeBringupRequestError {
    NonBringupCompileProfile {
        found: String,
    },
    UnsupportedTarget {
        found: String,
    },
    UnsupportedKernel,
    BadTile {
        found: TileSize,
    },
    UnsupportedFixture,
    SmokeLayoutRequiresN16 {
        found: u16,
    },
    SameBank {
        bank: u16,
    },
    BankZero,
    BankOutOfRange {
        bank: u16,
    },
    BadOperandOffset {
        operand: String,
        offset: u16,
        len: u32,
    },
    N128RequiresZeroOffset {
        operand: String,
        offset: u16,
    },
    Hash {
        reason: String,
    },
}

impl fmt::Display for ComputeBringupRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonBringupCompileProfile { found } => {
                write!(f, "compile profile {found} is not bringup")
            }
            Self::UnsupportedTarget { found } => {
                write!(
                    f,
                    "target profile {found} is not the F-B1 DMG/MBC5 bringup profile"
                )
            }
            Self::UnsupportedKernel => f.write_str("unsupported F-B1 kernel implementation"),
            Self::BadTile { found } => write!(
                f,
                "tile {}x{}x{} is not the fixed F-B1 16x16x16 tile",
                found.m, found.n, found.k
            ),
            Self::UnsupportedFixture => f.write_str("unsupported F-B1 operand fixture"),
            Self::SmokeLayoutRequiresN16 { found } => {
                write!(f, "WramSmoke layout requires N=16, found {found}")
            }
            Self::SameBank { bank } => write!(f, "A and B both use ROM bank {bank}"),
            Self::BankZero => f.write_str("operand ROM bank 0 is reserved"),
            Self::BankOutOfRange { bank } => write!(f, "ROM bank {bank} is outside MBC5 range"),
            Self::BadOperandOffset {
                operand,
                offset,
                len,
            } => write!(
                f,
                "operand {operand} at offset {offset} with length {len} does not fit in ROM bank"
            ),
            Self::N128RequiresZeroOffset { operand, offset } => {
                write!(
                    f,
                    "operand {operand} N=128 requires offset 0, found {offset}"
                )
            }
            Self::Hash { reason } => write!(f, "request hash failed: {reason}"),
        }
    }
}

impl std::error::Error for ComputeBringupRequestError {}

#[must_use]
pub fn materialize_operand_fixture(
    fixture: OperandFixtureSpec,
    operand: OperandKind,
    dim: SquareDim,
) -> Vec<u8> {
    match (fixture, operand) {
        (OperandFixtureSpec::DeterministicAffineV1, OperandKind::A) => {
            deterministic_operand(dim, |i, j| 73 * i + 37 * j + 19)
        }
        (OperandFixtureSpec::DeterministicAffineV1, OperandKind::B) => {
            deterministic_operand(dim, |i, j| 29 * i + 91 * j + 11)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OperandKind {
    A,
    B,
}

#[must_use]
pub fn quarter_square_table_i16() -> [i16; QUARTER_SQUARE_LEN] {
    let mut table = [0_i16; QUARTER_SQUARE_LEN];
    let mut x = QUARTER_SQUARE_MIN;
    while x <= QUARTER_SQUARE_MAX {
        let square = i32::from(x) * i32::from(x);
        table[quarter_square_index(x)] = (square / 4) as i16;
        x += 1;
    }
    table
}

#[must_use]
pub fn quarter_square_table_bytes_le() -> Vec<u8> {
    let mut bytes = Vec::with_capacity(QUARTER_SQUARE_BYTES as usize);
    for value in quarter_square_table_i16() {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

#[must_use]
pub fn quarter_square_table_split_bytes() -> Vec<u8> {
    let table = quarter_square_table_i16();
    let mut bytes = Vec::with_capacity(QUARTER_SQUARE_BYTES as usize);
    bytes.extend(table.iter().map(|value| value.to_le_bytes()[0]));
    bytes.extend(table.iter().map(|value| value.to_le_bytes()[1]));
    bytes
}

#[must_use]
pub fn quarter_square_mul_i8(a: i8, b: i8) -> i32 {
    static TABLE: std::sync::OnceLock<[i16; QUARTER_SQUARE_LEN]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(quarter_square_table_i16);
    let sum = i16::from(a) + i16::from(b);
    let diff = i16::from(a) - i16::from(b);
    i32::from(table[quarter_square_index(sum)]) - i32::from(table[quarter_square_index(diff)])
}

#[must_use]
pub fn quarter_square_index(x: i16) -> usize {
    usize::try_from(i32::from(x) - i32::from(QUARTER_SQUARE_MIN))
        .expect("quarter-square index is non-negative")
}

pub fn build_l0_wram_smoke_rom() -> Result<Vec<u8>, BringupRomBuildError> {
    let dim = SquareDim::new(16).expect("L0 shape is valid");
    let mut asm = FixedAsm::new(gbf_asm::rom::ENTRY_POINT);

    emit_fixture_load(
        &mut asm,
        FB1_L0_A_BASE,
        &materialize_operand_fixture(
            OperandFixtureSpec::DeterministicAffineV1,
            OperandKind::A,
            dim,
        ),
    );
    emit_fixture_load(
        &mut asm,
        FB1_L0_B_BASE,
        &materialize_operand_fixture(
            OperandFixtureSpec::DeterministicAffineV1,
            OperandKind::B,
            dim,
        ),
    );

    emit_l0_compute_main(&mut asm, dim);
    let program = asm.finish()?;
    assemble_program_rom(program, Vec::new(), "GBFB1L0", RomSize::Kib32)
}

pub fn build_l2_cross_bank_smoke_rom() -> Result<Vec<u8>, BringupRomBuildError> {
    let dim = SquareDim::new(16).expect("L2 smoke shape is valid");
    let mut asm = SymbolicAsm::new(FB1_L0_ROM_SECTION_ID, "l2_entry");
    emit_bankleased_romx_copy(&mut asm, FB1_L2_A_ROM_BANK, 0x4000, FB1_L0_A_BASE, 256)?;
    emit_bankleased_romx_copy(&mut asm, FB1_L2_B_ROM_BANK, 0x4000, FB1_L0_B_BASE, 256)?;
    emit_l0_compute_main(&mut asm, dim);

    let section = asm.finish();
    let a = materialize_operand_fixture(
        OperandFixtureSpec::DeterministicAffineV1,
        OperandKind::A,
        dim,
    );
    let b = materialize_operand_fixture(
        OperandFixtureSpec::DeterministicAffineV1,
        OperandKind::B,
        dim,
    );
    assemble_symbolic_program_rom(
        section,
        vec![
            RomExtraSection {
                id: SectionId::new(0xB102),
                bank: FB1_L2_A_ROM_BANK,
                cpu_start: 0x4000,
                bytes: a,
            },
            RomExtraSection {
                id: SectionId::new(0xB103),
                bank: FB1_L2_B_ROM_BANK,
                cpu_start: 0x4000,
                bytes: b,
            },
        ],
        "GBFB1L2",
        RomSize::Kib64,
    )
}

pub fn build_l3_partial_tile_rom(
    request: &ComputeBringupRequest,
    mt: u16,
    nt: u16,
    kt: u16,
) -> Result<Vec<u8>, BringupRomBuildError> {
    let (a_placement, b_placement) = distinct_bank_placements(request)?;
    let tiles = request.matrix_dim.tiles_per_axis();
    if mt >= tiles || nt >= tiles || kt >= tiles {
        return Err(BringupRomBuildError::pipeline(
            "tile ROM planning",
            format!("tile coordinate ({mt}, {nt}, {kt}) outside {tiles}x{tiles}x{tiles}"),
        ));
    }

    let dim = request.matrix_dim;
    let mut asm = SymbolicAsm::new(FB1_L0_ROM_SECTION_ID, "l3_partial_entry");
    emit_zero_accumulator_tile(&mut asm);
    emit_bankleased_panel_copy(
        &mut asm,
        a_placement.bank.get(),
        a_placement.offset.get(),
        OperandKind::A,
        dim,
        mt,
        kt,
    )?;
    emit_bankleased_panel_copy(
        &mut asm,
        b_placement.bank.get(),
        b_placement.offset.get(),
        OperandKind::B,
        dim,
        nt,
        kt,
    )?;
    emit_panel_accumulate_main(&mut asm, TILE_EDGE);
    asm.instr(Instr::Halt);
    emit_zero_i32_subroutine(&mut asm);
    emit_dot16_subroutine(&mut asm);
    emit_mul_add_subroutine(&mut asm);

    let section = asm.finish();
    let a = materialize_operand_fixture(request.operand_fixture, OperandKind::A, dim);
    let b = materialize_operand_fixture(request.operand_fixture, OperandKind::B, dim);
    assemble_symbolic_program_rom(
        section,
        vec![
            RomExtraSection {
                id: SectionId::new(0xB104),
                bank: a_placement.bank.get(),
                cpu_start: 0x4000 + a_placement.offset.get(),
                bytes: a,
            },
            RomExtraSection {
                id: SectionId::new(0xB105),
                bank: b_placement.bank.get(),
                cpu_start: 0x4000 + b_placement.offset.get(),
                bytes: b,
            },
        ],
        "GBFB1TILE",
        RomSize::Kib64,
    )
}

pub fn build_l3_output_tile_rom(
    request: &ComputeBringupRequest,
    mt: u16,
    nt: u16,
) -> Result<Vec<u8>, BringupRomBuildError> {
    let (a_placement, b_placement) = distinct_bank_placements(request)?;
    let dim = request.matrix_dim;
    let tiles = dim.tiles_per_axis();
    if mt >= tiles || nt >= tiles {
        return Err(BringupRomBuildError::pipeline(
            "tile ROM planning",
            format!("output tile coordinate ({mt}, {nt}) outside {tiles}x{tiles}"),
        ));
    }

    let mut asm = SymbolicAsm::new(FB1_L0_ROM_SECTION_ID, "l3_output_tile_entry");
    emit_zero_accumulator_tile(&mut asm);
    for kt in 0..tiles {
        emit_bankleased_panel_copy_looped(
            &mut asm,
            a_placement.bank.get(),
            a_placement.offset.get(),
            OperandKind::A,
            dim,
            mt,
            kt,
        )?;
        emit_bankleased_panel_copy_looped(
            &mut asm,
            b_placement.bank.get(),
            b_placement.offset.get(),
            OperandKind::B,
            dim,
            nt,
            kt,
        )?;
        asm.call("accumulate_panel");
    }
    asm.instr(Instr::Halt);
    emit_copy_count_subroutine(&mut asm);
    emit_accumulate_panel_subroutine(&mut asm);
    emit_zero_i32_subroutine(&mut asm);
    emit_dot16_subroutine(&mut asm);
    emit_mul_add_subroutine(&mut asm);

    let section = asm.finish();
    let a = materialize_operand_fixture(request.operand_fixture, OperandKind::A, dim);
    let b = materialize_operand_fixture(request.operand_fixture, OperandKind::B, dim);
    assemble_symbolic_program_rom(
        section,
        vec![
            RomExtraSection {
                id: SectionId::new(0xB106),
                bank: a_placement.bank.get(),
                cpu_start: 0x4000 + a_placement.offset.get(),
                bytes: a,
            },
            RomExtraSection {
                id: SectionId::new(0xB107),
                bank: b_placement.bank.get(),
                cpu_start: 0x4000 + b_placement.offset.get(),
                bytes: b,
            },
        ],
        "GBFB1OTIL",
        RomSize::Kib64,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BringupStreamingRom {
    pub rom: Vec<u8>,
    pub tile_safe_point_pc: u16,
    pub compute_yield_safe_point_pc: u16,
    pub copy_yield_safe_point_pc: u16,
    pub vblank_handler_pc: u16,
}

pub fn build_l3_streaming_rom(
    request: &ComputeBringupRequest,
) -> Result<BringupStreamingRom, BringupRomBuildError> {
    if request.yield_quantum != YieldQuantum::KLaneRow {
        return Err(BringupRomBuildError::pipeline(
            "streaming ROM planning",
            format!(
                "F-B1 streaming ROM emits KLaneRow safe points, found {:?}",
                request.yield_quantum
            ),
        ));
    }
    let (a_placement, b_placement) = distinct_bank_placements(request)?;
    let dim = request.matrix_dim;
    let mut asm = SymbolicAsm::new(FB1_L0_ROM_SECTION_ID, "l3_streaming_entry");
    emit_streaming_matmul_main(
        &mut asm,
        dim,
        a_placement.bank.get(),
        a_placement.offset.get(),
        b_placement.bank.get(),
        b_placement.offset.get(),
    )?;
    emit_copy_count_subroutine(&mut asm);
    emit_copy_panel_from_src_subroutine(&mut asm, OperandKind::A, FB1_L0_A_BASE, dim.n());
    emit_copy_panel_from_src_subroutine(&mut asm, OperandKind::B, FB1_L0_B_BASE, dim.n());
    emit_accumulate_panel_klane_row_subroutine(&mut asm);
    emit_zero_i32_subroutine(&mut asm);
    emit_dot16_subroutine(&mut asm);
    emit_mul_add_subroutine(&mut asm);
    emit_streaming_yield_subroutines(&mut asm);

    let section = asm.finish();
    let a = materialize_operand_fixture(request.operand_fixture, OperandKind::A, dim);
    let b = materialize_operand_fixture(request.operand_fixture, OperandKind::B, dim);

    let symbols = [
        f_b1_symbol("tile_safe_point"),
        f_b1_symbol("compute_yield_safe_point"),
        f_b1_symbol("copy_yield_safe_point"),
        f_b1_symbol("vblank_handler"),
    ];
    let (mut rom, symbol_addrs) = assemble_symbolic_program_rom_with_symbols(
        section,
        vec![
            RomExtraSection {
                id: SectionId::new(0xB108),
                bank: a_placement.bank.get(),
                cpu_start: 0x4000 + a_placement.offset.get(),
                bytes: a,
            },
            RomExtraSection {
                id: SectionId::new(0xB109),
                bank: b_placement.bank.get(),
                cpu_start: 0x4000 + b_placement.offset.get(),
                bytes: b,
            },
        ],
        "GBFB1STR",
        RomSize::Kib64,
        &symbols,
        vec![build_streaming_vblank_handler_section()],
        vec![PinnedPlacement {
            section_id: FB1_VBLANK_HANDLER_SECTION_ID,
            bank: BankIndex::Rom(0),
            cpu_start: 0x3000,
        }],
    )?;
    patch_vblank_vector(&mut rom, symbol_addrs[3])?;
    Ok(BringupStreamingRom {
        rom,
        tile_safe_point_pc: symbol_addrs[0],
        compute_yield_safe_point_pc: symbol_addrs[1],
        copy_yield_safe_point_pc: symbol_addrs[2],
        vblank_handler_pc: symbol_addrs[3],
    })
}

fn distinct_bank_placements(
    request: &ComputeBringupRequest,
) -> Result<(BankedOperandPlacement, BankedOperandPlacement), BringupRomBuildError> {
    request.validate().map_err(BringupRomBuildError::request)?;
    match &request.operand_layout {
        OperandLayout::DistinctRomBanks { a, b } => Ok((*a, *b)),
        OperandLayout::WramSmoke => Err(BringupRomBuildError::pipeline(
            "tile ROM planning",
            "L3 tile ROM requires distinct ROM banks",
        )),
    }
}

fn emit_bankleased_romx_copy(
    asm: &mut SymbolicAsm,
    bank: u16,
    source: u16,
    dest: u16,
    len: usize,
) -> Result<(), BringupRomBuildError> {
    let guard = lease_rom_switchable(
        &mut asm.builder,
        ValidatedBankLeaseSpec::for_rom_switchable(bank, LeaseLifetime::Slice)
            .map_err(BringupRomBuildError::banking)?,
    )
    .map_err(BringupRomBuildError::banking)?;
    copy_romx_to_wram_unrolled(asm, source, dest, len);
    release_bank(
        &mut asm.builder,
        guard,
        ReturnState::Rom(ReturnRomBank::Bank1),
    )
    .map_err(BringupRomBuildError::banking)?;
    Ok(())
}

fn emit_bankleased_panel_copy(
    asm: &mut SymbolicAsm,
    bank: u16,
    bank_offset: u16,
    operand: OperandKind,
    dim: SquareDim,
    tile_axis: u16,
    kt: u16,
) -> Result<(), BringupRomBuildError> {
    let guard = lease_rom_switchable(
        &mut asm.builder,
        ValidatedBankLeaseSpec::for_rom_switchable(bank, LeaseLifetime::Slice)
            .map_err(BringupRomBuildError::banking)?,
    )
    .map_err(BringupRomBuildError::banking)?;

    let n = dim.n();
    for row in 0..TILE_EDGE {
        let matrix_offset = match operand {
            OperandKind::A => {
                let matrix_row = tile_axis * TILE_EDGE + row;
                matrix_row * n + kt * TILE_EDGE
            }
            OperandKind::B => {
                let matrix_row = kt * TILE_EDGE + row;
                matrix_row * n + tile_axis * TILE_EDGE
            }
        };
        let source = 0x4000 + bank_offset + matrix_offset;
        let dest = match operand {
            OperandKind::A => FB1_L0_A_BASE + row * TILE_EDGE,
            OperandKind::B => FB1_L0_B_BASE + row * TILE_EDGE,
        };
        copy_romx_to_wram_unrolled(asm, source, dest, usize::from(TILE_EDGE));
    }

    release_bank(
        &mut asm.builder,
        guard,
        ReturnState::Rom(ReturnRomBank::Bank1),
    )
    .map_err(BringupRomBuildError::banking)?;
    Ok(())
}

fn emit_bankleased_panel_copy_looped(
    asm: &mut SymbolicAsm,
    bank: u16,
    bank_offset: u16,
    operand: OperandKind,
    dim: SquareDim,
    tile_axis: u16,
    kt: u16,
) -> Result<(), BringupRomBuildError> {
    let guard = lease_rom_switchable(
        &mut asm.builder,
        ValidatedBankLeaseSpec::for_rom_switchable(bank, LeaseLifetime::Slice)
            .map_err(BringupRomBuildError::banking)?,
    )
    .map_err(BringupRomBuildError::banking)?;

    let n = dim.n();
    for row in 0..TILE_EDGE {
        let matrix_offset = match operand {
            OperandKind::A => {
                let matrix_row = tile_axis * TILE_EDGE + row;
                matrix_row * n + kt * TILE_EDGE
            }
            OperandKind::B => {
                let matrix_row = kt * TILE_EDGE + row;
                matrix_row * n + tile_axis * TILE_EDGE
            }
        };
        let source = 0x4000 + bank_offset + matrix_offset;
        let dest = match operand {
            OperandKind::A => FB1_L0_A_BASE + row * TILE_EDGE,
            OperandKind::B => FB1_L0_B_BASE + row * TILE_EDGE,
        };
        write_ptr(asm, L0_COPY_SRC_PTR, source);
        write_ptr(asm, L0_COPY_DST_PTR, dest);
        write_direct_imm(asm, L0_COPY_COUNT, TILE_EDGE as u8);
        asm.call("copy_count_bytes");
    }

    release_bank(
        &mut asm.builder,
        guard,
        ReturnState::Rom(ReturnRomBank::Bank1),
    )
    .map_err(BringupRomBuildError::banking)?;
    Ok(())
}

trait MatmulAsm {
    fn instr(&mut self, instr: Instr);
    fn label(&mut self, label: &'static str);
    fn jump(&mut self, cond: Option<Cond>, label: &'static str);
    fn call(&mut self, label: &'static str);
}

fn emit_l0_compute_main(asm: &mut impl MatmulAsm, dim: SquareDim) {
    debug_assert_eq!(dim.n(), TILE_EDGE);
    emit_zero_accumulator_tile(asm);
    emit_panel_accumulate_main(asm, dim.n());
    asm.instr(Instr::Halt);

    emit_zero_i32_subroutine(asm);
    emit_dot16_subroutine(asm);
    emit_mul_add_subroutine(asm);
}

fn emit_zero_accumulator_tile(asm: &mut impl MatmulAsm) {
    for i in 0..TILE_EDGE {
        for j in 0..TILE_EDGE {
            write_ptr(
                asm,
                L0_OUT_PTR,
                FB1_L0_OUTPUT_BASE + ((i * TILE_EDGE + j) * 4),
            );
            asm.call("zero_i32_at_out_ptr");
        }
    }
}

fn emit_panel_accumulate_main(asm: &mut impl MatmulAsm, panel_stride: u16) {
    for i in 0..TILE_EDGE {
        for j in 0..TILE_EDGE {
            write_ptr(asm, L0_A_PTR, FB1_L0_A_BASE + (i * panel_stride));
            write_ptr(asm, L0_B_PTR, FB1_L0_B_BASE + j);
            write_ptr(
                asm,
                L0_OUT_PTR,
                FB1_L0_OUTPUT_BASE + ((i * TILE_EDGE + j) * 4),
            );
            asm.call("dot16");
        }
    }
}

#[derive(Debug)]
pub enum BringupRomBuildError {
    DuplicateLabel(&'static str),
    UndefinedLabel(&'static str),
    ProgramTooLarge { len: usize },
    AsmPipeline { stage: &'static str, reason: String },
    Encode(EncodeError),
    Rom(RomAssemblyError),
}

impl fmt::Display for BringupRomBuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateLabel(label) => write!(f, "duplicate F-B1 ROM label {label}"),
            Self::UndefinedLabel(label) => write!(f, "undefined F-B1 ROM label {label}"),
            Self::ProgramTooLarge { len } => {
                write!(f, "F-B1 L0 ROM program is too large: {len} bytes")
            }
            Self::AsmPipeline { stage, reason } => {
                write!(f, "F-B1 ROM {stage} failed: {reason}")
            }
            Self::Encode(error) => write!(f, "F-B1 L0 ROM encode failed: {error}"),
            Self::Rom(error) => write!(f, "F-B1 L0 ROM assembly failed: {error}"),
        }
    }
}

impl std::error::Error for BringupRomBuildError {}

impl From<EncodeError> for BringupRomBuildError {
    fn from(error: EncodeError) -> Self {
        Self::Encode(error)
    }
}

impl From<RomAssemblyError> for BringupRomBuildError {
    fn from(error: RomAssemblyError) -> Self {
        Self::Rom(error)
    }
}

impl BringupRomBuildError {
    fn pipeline(stage: &'static str, error: impl fmt::Display) -> Self {
        Self::AsmPipeline {
            stage,
            reason: error.to_string(),
        }
    }

    fn banking(error: impl fmt::Display) -> Self {
        Self::pipeline("banking", error)
    }

    fn request(error: impl fmt::Display) -> Self {
        Self::pipeline("request validation", error)
    }
}

const L0_A_PTR: u16 = 0xC700;
const L0_B_PTR: u16 = 0xC702;
const L0_OUT_PTR: u16 = 0xC704;
const L0_K_COUNT: u16 = 0xC706;
const L0_TEMP_A: u16 = 0xC707;
const L0_TEMP_B: u16 = 0xC708;
const L0_PROD_LO: u16 = 0xC709;
const L0_PROD_HI: u16 = 0xC70A;
const L0_COPY_SRC_PTR: u16 = 0xC70B;
const L0_COPY_DST_PTR: u16 = 0xC70D;
const L0_COPY_COUNT: u16 = 0xC70F;
const L0_I_COUNT: u16 = 0xC710;
const L0_J_COUNT: u16 = 0xC711;
const L0_A_ROW_PTR: u16 = 0xC712;
const L0_B_COL_PTR: u16 = 0xC714;
const L0_OUT_WALK_PTR: u16 = 0xC716;
const L0_MT_COUNT: u16 = 0xC718;
const L0_NT_COUNT: u16 = 0xC719;
const L0_KT_COUNT: u16 = 0xC71A;
const L0_COPY_ROW_COUNT: u16 = 0xC71B;
const L0_A_KT_SRC_PTR: u16 = 0xC71C;
const L0_B_KT_SRC_PTR: u16 = 0xC71E;
const L0_MT_A_BASE_PTR: u16 = 0xC720;
const L0_NT_B_BASE_PTR: u16 = 0xC722;
const L0_SIGN_FLAG: u16 = 0xC724;
const L0_MUL_COUNT: u16 = 0xC725;
const L0_A_KLANE_BASE_PTR: u16 = 0xC726;
const L0_B_KLANE_BASE_PTR: u16 = 0xC728;
const L4_LAST_SERVICED_FRAME_LDH: u8 = 0x88;
const L4_WIDGET_UPDATE_COUNT_LDH: u8 = 0x89;
const L4_SCHEDULER_SERVICE_COUNT_LDH: u8 = 0x8A;
const F_B1_VBLANK_VECTOR_ADDR: usize = gbf_hw::interrupts::INT_VECTOR_VBLANK as usize;

enum FixedAsmOp {
    Instr(Instr),
    Label(&'static str),
    Jump {
        cond: Option<Cond>,
        label: &'static str,
    },
    Call(&'static str),
}

struct FixedAsm {
    start: u16,
    ops: Vec<FixedAsmOp>,
}

impl FixedAsm {
    fn new(start: u16) -> Self {
        Self {
            start,
            ops: Vec::new(),
        }
    }

    fn finish(&self) -> Result<Vec<u8>, BringupRomBuildError> {
        let mut labels = std::collections::BTreeMap::new();
        let mut pc = self.start;
        for op in &self.ops {
            match op {
                FixedAsmOp::Instr(instr) => {
                    pc = pc.wrapping_add(u16::from(instr.byte_len()));
                }
                FixedAsmOp::Jump { .. } | FixedAsmOp::Call(_) => {
                    pc = pc.wrapping_add(3);
                }
                FixedAsmOp::Label(label) => {
                    if labels.insert(*label, pc).is_some() {
                        return Err(BringupRomBuildError::DuplicateLabel(label));
                    }
                }
            }
        }

        let mut out = Vec::new();
        for op in &self.ops {
            match op {
                FixedAsmOp::Instr(instr) => out.extend_from_slice(&encode_instr(instr)?),
                FixedAsmOp::Jump { cond, label } => {
                    let addr = *labels
                        .get(label)
                        .ok_or(BringupRomBuildError::UndefinedLabel(label))?;
                    out.extend_from_slice(&encode_instr(&Instr::JpAbs { cond: *cond, addr })?);
                }
                FixedAsmOp::Call(label) => {
                    let addr = *labels
                        .get(label)
                        .ok_or(BringupRomBuildError::UndefinedLabel(label))?;
                    out.extend_from_slice(&encode_instr(&Instr::Call { cond: None, addr })?);
                }
                FixedAsmOp::Label(_) => {}
            }
        }
        Ok(out)
    }
}

impl MatmulAsm for FixedAsm {
    fn instr(&mut self, instr: Instr) {
        self.ops.push(FixedAsmOp::Instr(instr));
    }

    fn label(&mut self, label: &'static str) {
        self.ops.push(FixedAsmOp::Label(label));
    }

    fn jump(&mut self, cond: Option<Cond>, label: &'static str) {
        self.ops.push(FixedAsmOp::Jump { cond, label });
    }

    fn call(&mut self, label: &'static str) {
        self.ops.push(FixedAsmOp::Call(label));
    }
}

struct SymbolicAsm {
    builder: Builder,
}

impl SymbolicAsm {
    fn new(id: SectionId, entry: &'static str) -> Self {
        let builder = Builder::new_with_id(id, SectionRole::Bank0Nucleus, f_b1_symbol(entry))
            .with_section_privilege(SectionPrivilege::privileged());
        Self { builder }
    }

    fn finish(self) -> Section {
        self.builder.finish()
    }
}

impl MatmulAsm for SymbolicAsm {
    fn instr(&mut self, instr: Instr) {
        self.builder.emit(instr);
    }

    fn label(&mut self, label: &'static str) {
        self.builder.label(f_b1_symbol(label));
    }

    fn jump(&mut self, cond: Option<Cond>, label: &'static str) {
        self.builder.branch(SymbolicBranch::new(
            BranchKind::Jump,
            cond,
            f_b1_symbol(label),
        ));
    }

    fn call(&mut self, label: &'static str) {
        self.builder
            .branch(SymbolicBranch::call(f_b1_symbol(label), None));
    }
}

fn f_b1_symbol(label: &'static str) -> SymbolName {
    SymbolName::runtime("f_b1", label).expect("F-B1 static symbol is valid")
}

fn emit_fixture_load(asm: &mut impl MatmulAsm, base: u16, bytes: &[u8]) {
    for (offset, value) in bytes.iter().copied().enumerate() {
        write_direct_imm(
            asm,
            base + u16::try_from(offset).expect("fixture fits WRAM"),
            value,
        );
    }
}

fn emit_copy_count_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("copy_count_bytes");
    load_hl_from_ptr(asm, L0_COPY_SRC_PTR);
    load_de_from_ptr(asm, L0_COPY_DST_PTR);
    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_COPY_COUNT),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    asm.label("copy_count_loop");
    asm.instr(Instr::LdAFromReg16Addr {
        src: Reg16Addr::Hli,
    });
    asm.instr(Instr::LdReg16AddrFromA { dst: Reg16Addr::DE });
    asm.instr(Instr::Inc16 { dst: Reg16Data::DE });
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::C),
    });
    asm.jump(Some(Cond::NZ), "copy_count_loop");
    asm.instr(Instr::Ret { cond: None });
}

fn emit_streaming_matmul_main(
    asm: &mut SymbolicAsm,
    dim: SquareDim,
    a_bank: u16,
    a_bank_offset: u16,
    b_bank: u16,
    b_bank_offset: u16,
) -> Result<(), BringupRomBuildError> {
    let tiles = dim.tiles_per_axis();
    let n = dim.n();
    let tile_stride = TILE_EDGE * n;

    emit_streaming_runtime_setup(asm);

    write_ptr(asm, L0_MT_A_BASE_PTR, 0x4000 + a_bank_offset);
    write_direct_imm(asm, L0_MT_COUNT, tiles as u8);

    asm.label("stream_mt_loop");
    write_ptr(asm, L0_NT_B_BASE_PTR, 0x4000 + b_bank_offset);
    write_direct_imm(asm, L0_NT_COUNT, tiles as u8);

    asm.label("stream_nt_loop");
    emit_zero_accumulator_tile(asm);
    copy_ptr(asm, L0_MT_A_BASE_PTR, L0_A_KT_SRC_PTR);
    copy_ptr(asm, L0_NT_B_BASE_PTR, L0_B_KT_SRC_PTR);
    write_direct_imm(asm, L0_KT_COUNT, tiles as u8);

    asm.label("stream_kt_loop");
    copy_ptr(asm, L0_A_KT_SRC_PTR, L0_COPY_SRC_PTR);
    emit_bankleased_panel_copy_from_current_src(asm, a_bank, OperandKind::A)?;
    copy_ptr(asm, L0_B_KT_SRC_PTR, L0_COPY_SRC_PTR);
    emit_bankleased_panel_copy_from_current_src(asm, b_bank, OperandKind::B)?;
    asm.call("accumulate_panel");
    add_ptr_imm(asm, L0_A_KT_SRC_PTR, TILE_EDGE);
    add_ptr_imm(asm, L0_B_KT_SRC_PTR, tile_stride);
    dec_direct_and_jump(asm, L0_KT_COUNT, "stream_kt_loop");

    asm.label("tile_safe_point");
    asm.instr(Instr::Nop);
    add_ptr_imm(asm, L0_NT_B_BASE_PTR, TILE_EDGE);
    dec_direct_and_jump(asm, L0_NT_COUNT, "stream_nt_loop");
    add_ptr_imm(asm, L0_MT_A_BASE_PTR, tile_stride);
    dec_direct_and_jump(asm, L0_MT_COUNT, "stream_mt_loop");
    asm.label("stream_done_loop");
    asm.instr(Instr::Halt);
    asm.jump(None, "stream_done_loop");
    Ok(())
}

fn emit_streaming_runtime_setup(asm: &mut impl MatmulAsm) {
    asm.instr(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    for offset in [
        gbf_runtime::scheduler::HRAM_LDH_FRAME_COUNT,
        L4_LAST_SERVICED_FRAME_LDH,
        L4_WIDGET_UPDATE_COUNT_LDH,
        L4_SCHEDULER_SERVICE_COUNT_LDH,
    ] {
        asm.instr(Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(offset),
        });
    }
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: gbf_hw::interrupts::ie_bit(gbf_hw::interrupts::InterruptSource::VBlank),
    });
    asm.instr(Instr::LdHighDirectFromA {
        offset: high_direct(gbf_hw::interrupts::IE_REGISTER),
    });
    asm.instr(Instr::Ei);
}

fn copy_panel_subroutine_label(operand: OperandKind) -> &'static str {
    match operand {
        OperandKind::A => "copy_a_panel_from_src",
        OperandKind::B => "copy_b_panel_from_src",
    }
}

fn copy_panel_row_loop_label(operand: OperandKind) -> &'static str {
    match operand {
        OperandKind::A => "copy_a_panel_row_loop",
        OperandKind::B => "copy_b_panel_row_loop",
    }
}

fn emit_bankleased_panel_copy_from_current_src(
    asm: &mut SymbolicAsm,
    bank: u16,
    operand: OperandKind,
) -> Result<(), BringupRomBuildError> {
    let guard = lease_rom_switchable(
        &mut asm.builder,
        ValidatedBankLeaseSpec::for_rom_switchable(bank, LeaseLifetime::Slice)
            .map_err(BringupRomBuildError::banking)?,
    )
    .map_err(BringupRomBuildError::banking)?;
    asm.call(copy_panel_subroutine_label(operand));
    release_bank(
        &mut asm.builder,
        guard,
        ReturnState::Rom(ReturnRomBank::Bank1),
    )
    .map_err(BringupRomBuildError::banking)?;
    asm.call("copy_yield_safe_point");
    Ok(())
}

fn emit_copy_panel_from_src_subroutine(
    asm: &mut impl MatmulAsm,
    operand: OperandKind,
    dest_base: u16,
    row_stride: u16,
) {
    let row_loop = copy_panel_row_loop_label(operand);

    asm.label(copy_panel_subroutine_label(operand));
    write_ptr(asm, L0_COPY_DST_PTR, dest_base);
    write_direct_imm(asm, L0_COPY_ROW_COUNT, TILE_EDGE as u8);

    asm.label(row_loop);
    write_direct_imm(asm, L0_COPY_COUNT, TILE_EDGE as u8);
    asm.call("copy_count_bytes");
    add_ptr_imm(asm, L0_COPY_SRC_PTR, row_stride);
    add_ptr_imm(asm, L0_COPY_DST_PTR, TILE_EDGE);
    dec_direct_and_jump(asm, L0_COPY_ROW_COUNT, row_loop);
    asm.instr(Instr::Ret { cond: None });
}

fn emit_accumulate_panel_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("accumulate_panel");
    write_ptr(asm, L0_A_ROW_PTR, FB1_L0_A_BASE);
    write_ptr(asm, L0_OUT_WALK_PTR, FB1_L0_OUTPUT_BASE);
    write_direct_imm(asm, L0_I_COUNT, TILE_EDGE as u8);

    asm.label("accumulate_row_loop");
    write_direct_imm(asm, L0_J_COUNT, TILE_EDGE as u8);
    write_ptr(asm, L0_B_COL_PTR, FB1_L0_B_BASE);

    asm.label("accumulate_col_loop");
    copy_ptr(asm, L0_A_ROW_PTR, L0_A_PTR);
    copy_ptr(asm, L0_B_COL_PTR, L0_B_PTR);
    copy_ptr(asm, L0_OUT_WALK_PTR, L0_OUT_PTR);
    asm.call("dot16");
    add_ptr_imm(asm, L0_B_COL_PTR, 1);
    add_ptr_imm(asm, L0_OUT_WALK_PTR, 4);
    dec_direct_and_jump(asm, L0_J_COUNT, "accumulate_col_loop");

    add_ptr_imm(asm, L0_A_ROW_PTR, TILE_EDGE);
    dec_direct_and_jump(asm, L0_I_COUNT, "accumulate_row_loop");
    asm.instr(Instr::Ret { cond: None });
}

fn emit_accumulate_panel_klane_row_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("accumulate_panel");
    write_ptr(asm, L0_A_KLANE_BASE_PTR, FB1_L0_A_BASE);
    write_ptr(asm, L0_B_KLANE_BASE_PTR, FB1_L0_B_BASE);
    write_direct_imm(asm, L0_K_COUNT, TILE_EDGE as u8);

    asm.label("accumulate_klane_loop");
    copy_ptr(asm, L0_A_KLANE_BASE_PTR, L0_A_ROW_PTR);
    copy_ptr(asm, L0_B_KLANE_BASE_PTR, L0_B_COL_PTR);
    write_ptr(asm, L0_OUT_WALK_PTR, FB1_L0_OUTPUT_BASE);
    write_direct_imm(asm, L0_I_COUNT, TILE_EDGE as u8);

    asm.label("accumulate_klane_row_loop");
    copy_ptr(asm, L0_A_ROW_PTR, L0_A_PTR);
    copy_ptr(asm, L0_B_COL_PTR, L0_B_PTR);
    copy_ptr(asm, L0_OUT_WALK_PTR, L0_OUT_PTR);
    write_direct_imm(asm, L0_J_COUNT, TILE_EDGE as u8);

    asm.label("accumulate_klane_col_loop");
    load_hl_from_ptr(asm, L0_A_PTR);
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_TEMP_A),
    });

    load_hl_from_ptr(asm, L0_B_PTR);
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_TEMP_B),
    });
    asm.instr(Instr::Inc16 { dst: Reg16Data::HL });
    store_hl_to_ptr(asm, L0_B_PTR);

    asm.call("mul_add_i8_i32");
    add_ptr_imm(asm, L0_OUT_PTR, 4);
    dec_direct_and_jump(asm, L0_J_COUNT, "accumulate_klane_col_loop");

    asm.call("compute_yield_safe_point");
    add_ptr_imm(asm, L0_A_ROW_PTR, TILE_EDGE);
    add_ptr_imm(asm, L0_OUT_WALK_PTR, TILE_EDGE * 4);
    dec_direct_and_jump(asm, L0_I_COUNT, "accumulate_klane_row_loop");

    add_ptr_imm(asm, L0_A_KLANE_BASE_PTR, 1);
    add_ptr_imm(asm, L0_B_KLANE_BASE_PTR, TILE_EDGE);
    dec_direct_and_jump(asm, L0_K_COUNT, "accumulate_klane_loop");
    asm.instr(Instr::Ret { cond: None });
}

fn emit_streaming_yield_subroutines(asm: &mut impl MatmulAsm) {
    asm.label("compute_yield_safe_point");
    asm.instr(Instr::Nop);
    emit_rom_side_frame_service(asm, "frame_service_done_compute");
    asm.instr(Instr::Ret { cond: None });

    asm.label("copy_yield_safe_point");
    asm.instr(Instr::Nop);
    asm.instr(Instr::Nop);
    emit_rom_side_frame_service(asm, "frame_service_done_copy");
    asm.instr(Instr::Ret { cond: None });
}

fn build_streaming_vblank_handler_section() -> Section {
    let mut builder = Builder::new_with_id(
        FB1_VBLANK_HANDLER_SECTION_ID,
        SectionRole::Bank0Nucleus,
        f_b1_symbol("vblank_handler_section"),
    )
    .with_section_privilege(SectionPrivilege::interrupt_handler());
    builder.label(f_b1_symbol("vblank_handler"));
    for reg in [
        Reg16Stack::AF,
        Reg16Stack::BC,
        Reg16Stack::DE,
        Reg16Stack::HL,
    ] {
        builder.emit(Instr::Push { src: reg });
    }
    gbf_runtime::interrupts::emit_vblank_handler(&mut builder);
    for reg in [
        Reg16Stack::HL,
        Reg16Stack::DE,
        Reg16Stack::BC,
        Reg16Stack::AF,
    ] {
        builder.emit(Instr::Pop { dst: reg });
    }
    builder.emit(Instr::Reti);
    builder
        .label(SymbolName::runtime("video_commit", "drain_vblank").expect("static runtime symbol"));
    builder.emit(Instr::Ret { cond: None });
    builder.finish().with_size_hint_bytes(96)
}

fn emit_rom_side_frame_service(asm: &mut impl MatmulAsm, done_label: &'static str) {
    asm.instr(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(gbf_runtime::scheduler::HRAM_LDH_FRAME_COUNT),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    asm.instr(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(L4_LAST_SERVICED_FRAME_LDH),
    });
    asm.instr(Instr::CpA {
        src: AluSrc8::Reg(Reg8::B),
    });
    asm.jump(Some(Cond::Z), done_label);
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::B,
    });
    asm.instr(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(L4_LAST_SERVICED_FRAME_LDH),
    });
    asm.instr(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(L4_WIDGET_UPDATE_COUNT_LDH),
    });
    asm.instr(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    asm.instr(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(L4_WIDGET_UPDATE_COUNT_LDH),
    });
    asm.instr(Instr::LdAFromHighDirect {
        offset: HighDirectOffset::new(L4_SCHEDULER_SERVICE_COUNT_LDH),
    });
    asm.instr(Instr::Inc8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    asm.instr(Instr::LdHighDirectFromA {
        offset: HighDirectOffset::new(L4_SCHEDULER_SERVICE_COUNT_LDH),
    });
    asm.label(done_label);
}

fn emit_zero_i32_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("zero_i32_at_out_ptr");
    load_hl_from_ptr(asm, L0_OUT_PTR);
    asm.instr(Instr::XorA {
        src: AluSrc8::Reg(Reg8::A),
    });
    for _ in 0..4 {
        asm.instr(Instr::Ld8HlFromReg { src: Reg8::A });
        asm.instr(Instr::Inc16 { dst: Reg16Data::HL });
    }
    asm.instr(Instr::Ret { cond: None });
}

fn emit_dot16_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("dot16");
    write_direct_imm(asm, L0_K_COUNT, 16);

    asm.label("dot16_loop");
    load_hl_from_ptr(asm, L0_A_PTR);
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_TEMP_A),
    });
    asm.instr(Instr::Inc16 { dst: Reg16Data::HL });
    store_hl_to_ptr(asm, L0_A_PTR);

    load_hl_from_ptr(asm, L0_B_PTR);
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_TEMP_B),
    });
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::BC,
        imm: 16,
    });
    asm.instr(Instr::AddHl { src: Reg16Data::BC });
    store_hl_to_ptr(asm, L0_B_PTR);

    asm.call("mul_add_i8_i32");
    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_K_COUNT),
    });
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_K_COUNT),
    });
    asm.jump(Some(Cond::NZ), "dot16_loop");
    asm.instr(Instr::Ret { cond: None });
}

fn emit_mul_add_subroutine(asm: &mut impl MatmulAsm) {
    asm.label("mul_add_i8_i32");
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::D,
        imm: 0,
    });

    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_TEMP_A),
    });
    asm.instr(Instr::Bit {
        bit: BitIndex::B7,
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jump(Some(Cond::Z), "mul_a_positive");
    asm.instr(Instr::Cpl);
    asm.instr(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    toggle_sign(asm);
    asm.jump(None, "mul_a_done");
    asm.label("mul_a_positive");
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    asm.label("mul_a_done");

    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_TEMP_B),
    });
    asm.instr(Instr::Bit {
        bit: BitIndex::B7,
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jump(Some(Cond::Z), "mul_b_positive");
    asm.instr(Instr::Cpl);
    asm.instr(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    toggle_sign(asm);
    asm.jump(None, "mul_b_done");
    asm.label("mul_b_positive");
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::B,
        src: Reg8::A,
    });
    asm.label("mul_b_done");

    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_SIGN_FLAG),
    });
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::H,
        imm: 0,
    });
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::L,
        imm: 0,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::H,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::C,
    });
    write_direct_imm(asm, L0_MUL_COUNT, 8);

    asm.label("mul_shift_loop");
    asm.instr(Instr::Bit {
        bit: BitIndex::B0,
        target: CbTarget::Reg(Reg8::B),
    });
    asm.jump(Some(Cond::Z), "mul_shift_skip_add");

    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    asm.instr(Instr::AddA {
        src: AluSrc8::Reg(Reg8::E),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::D),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::H,
        src: Reg8::A,
    });

    asm.label("mul_shift_skip_add");
    asm.instr(Instr::Sla {
        target: CbTarget::Reg(Reg8::E),
    });
    asm.instr(Instr::Rl {
        target: CbTarget::Reg(Reg8::D),
    });
    asm.instr(Instr::Srl {
        target: CbTarget::Reg(Reg8::B),
    });
    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_MUL_COUNT),
    });
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_MUL_COUNT),
    });
    asm.jump(Some(Cond::NZ), "mul_shift_loop");

    asm.label("mul_loop_done");
    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_SIGN_FLAG),
    });
    asm.instr(Instr::CpA {
        src: AluSrc8::Imm(0),
    });
    asm.jump(Some(Cond::Z), "mul_product_signed");
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    asm.instr(Instr::Cpl);
    asm.instr(Instr::AddA {
        src: AluSrc8::Imm(1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    asm.instr(Instr::Cpl);
    asm.instr(Instr::AdcA {
        src: AluSrc8::Imm(0),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::H,
        src: Reg8::A,
    });

    asm.label("mul_product_signed");
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::E,
        imm: 0,
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    asm.instr(Instr::Bit {
        bit: BitIndex::B7,
        target: CbTarget::Reg(Reg8::A),
    });
    asm.jump(Some(Cond::Z), "mul_sign_ext_done");
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::E,
        imm: 0xFF,
    });
    asm.label("mul_sign_ext_done");

    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_PROD_LO),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(L0_PROD_HI),
    });
    load_hl_from_ptr(asm, L0_OUT_PTR);

    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_PROD_LO),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::AddA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.instr(Instr::Inc16 { dst: Reg16Data::HL });

    asm.instr(Instr::LdAFromDirect {
        addr: direct(L0_PROD_HI),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::A,
    });
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.instr(Instr::Inc16 { dst: Reg16Data::HL });

    asm.instr(Instr::Ld8Reg {
        dst: Reg8::C,
        src: Reg8::E,
    });
    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.instr(Instr::Inc16 { dst: Reg16Data::HL });

    asm.instr(Instr::Ld8RegFromHl { dst: Reg8::A });
    asm.instr(Instr::AdcA {
        src: AluSrc8::Reg(Reg8::C),
    });
    asm.instr(Instr::Ld8HlFromReg { src: Reg8::A });
    asm.instr(Instr::Ret { cond: None });
}

fn toggle_sign(asm: &mut impl MatmulAsm) {
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::D,
    });
    asm.instr(Instr::XorA {
        src: AluSrc8::Imm(1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
}

fn load_hl_from_ptr(asm: &mut impl MatmulAsm, ptr: u16) {
    asm.instr(Instr::LdAFromDirect {
        addr: direct(ptr + 1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::H,
        src: Reg8::A,
    });
    asm.instr(Instr::LdAFromDirect { addr: direct(ptr) });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::L,
        src: Reg8::A,
    });
}

fn load_de_from_ptr(asm: &mut impl MatmulAsm, ptr: u16) {
    asm.instr(Instr::LdAFromDirect {
        addr: direct(ptr + 1),
    });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::D,
        src: Reg8::A,
    });
    asm.instr(Instr::LdAFromDirect { addr: direct(ptr) });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::E,
        src: Reg8::A,
    });
}

fn store_hl_to_ptr(asm: &mut impl MatmulAsm, ptr: u16) {
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::L,
    });
    asm.instr(Instr::LdDirectFromA { addr: direct(ptr) });
    asm.instr(Instr::Ld8Reg {
        dst: Reg8::A,
        src: Reg8::H,
    });
    asm.instr(Instr::LdDirectFromA {
        addr: direct(ptr + 1),
    });
}

fn copy_ptr(asm: &mut impl MatmulAsm, src: u16, dst: u16) {
    load_hl_from_ptr(asm, src);
    store_hl_to_ptr(asm, dst);
}

fn add_ptr_imm(asm: &mut impl MatmulAsm, ptr: u16, value: u16) {
    load_hl_from_ptr(asm, ptr);
    asm.instr(Instr::Ld16Imm {
        dst: Reg16Data::BC,
        imm: value,
    });
    asm.instr(Instr::AddHl { src: Reg16Data::BC });
    store_hl_to_ptr(asm, ptr);
}

fn dec_direct_and_jump(asm: &mut impl MatmulAsm, ptr: u16, label: &'static str) {
    asm.instr(Instr::LdAFromDirect { addr: direct(ptr) });
    asm.instr(Instr::Dec8 {
        dst: IncDec8Target::Reg(Reg8::A),
    });
    asm.instr(Instr::LdDirectFromA { addr: direct(ptr) });
    asm.jump(Some(Cond::NZ), label);
}

fn write_ptr(asm: &mut impl MatmulAsm, ptr: u16, value: u16) {
    write_direct_imm(asm, ptr, value as u8);
    write_direct_imm(asm, ptr + 1, (value >> 8) as u8);
}

fn write_direct_imm(asm: &mut impl MatmulAsm, addr: u16, value: u8) {
    asm.instr(Instr::Ld8RegFromImm {
        dst: Reg8::A,
        imm: value,
    });
    asm.instr(Instr::LdDirectFromA { addr: direct(addr) });
}

fn copy_romx_to_wram_unrolled(asm: &mut impl MatmulAsm, source: u16, dest: u16, len: usize) {
    for offset in 0..len {
        let offset = u16::try_from(offset).expect("copy span fits u16");
        asm.instr(Instr::LdAFromDirect {
            addr: direct(source + offset),
        });
        asm.instr(Instr::LdDirectFromA {
            addr: direct(dest + offset),
        });
    }
}

fn direct(addr: u16) -> DirectAddr {
    DirectAddr::new(addr).expect("F-B1 L0 ROM uses non-high-memory direct addresses")
}

fn high_direct(addr: u16) -> HighDirectOffset {
    assert!(
        (addr & 0xFF00) == 0xFF00,
        "LDH direct offset requires a high-memory address"
    );
    HighDirectOffset::new((addr & 0x00FF) as u8)
}

fn patch_vblank_vector(rom: &mut [u8], handler_pc: u16) -> Result<(), BringupRomBuildError> {
    let jump = encode_instr(&Instr::JpAbs {
        cond: None,
        addr: handler_pc,
    })?;
    let vector = &mut rom[F_B1_VBLANK_VECTOR_ADDR..F_B1_VBLANK_VECTOR_ADDR + 8];
    vector.fill(0);
    vector[..jump.len()].copy_from_slice(&jump);
    let checksum = global_checksum(rom);
    rom[0x014E] = (checksum >> 8) as u8;
    rom[0x014F] = (checksum & 0x00FF) as u8;
    Ok(())
}

struct RomExtraSection {
    id: SectionId,
    bank: u16,
    cpu_start: u16,
    bytes: Vec<u8>,
}

fn assemble_program_rom(
    program: Vec<u8>,
    extra: Vec<RomExtraSection>,
    title: &str,
    rom_size: RomSize,
) -> Result<Vec<u8>, BringupRomBuildError> {
    if program.len() > usize::from(ROM0_END_EXCLUSIVE - gbf_asm::rom::ENTRY_POINT) {
        return Err(BringupRomBuildError::ProgramTooLarge { len: program.len() });
    }
    let mut pairs = Vec::with_capacity(1 + extra.len());
    let encoded = EncodedSection {
        id: FB1_L0_ROM_SECTION_ID,
        bytes: program,
        item_spans: Vec::new(),
    };
    let placed = PlacedSection {
        id: FB1_L0_ROM_SECTION_ID,
        space: AddressSpace::Rom0,
        bank: BankIndex::Rom(0),
        cpu_start: gbf_asm::rom::ENTRY_POINT,
        final_size: u16::try_from(encoded.bytes.len()).expect("program size checked"),
        estimated_size: u16::try_from(encoded.bytes.len()).expect("program size checked"),
        alignment_padding: Default::default(),
    };
    pairs.push((encoded, placed.clone()));
    let mut sections = vec![placed];
    for section in extra {
        let encoded = EncodedSection {
            id: section.id,
            bytes: section.bytes,
            item_spans: Vec::new(),
        };
        let placed = PlacedSection {
            id: section.id,
            space: AddressSpace::RomX,
            bank: BankIndex::Rom(section.bank),
            cpu_start: section.cpu_start,
            final_size: u16::try_from(encoded.bytes.len()).expect("extra section fits u16"),
            estimated_size: u16::try_from(encoded.bytes.len()).expect("extra section fits u16"),
            alignment_padding: Default::default(),
        };
        sections.push(placed.clone());
        pairs.push((encoded, placed));
    }
    let layout = LayoutPlan {
        sections,
        bank_count: rom_size.bank_count(),
        free_bytes_per_bank: Default::default(),
        reserved_ranges: Vec::new(),
    };
    let mut header = CartridgeHeader::new(title)?;
    header.rom_size = rom_size;
    Ok(assemble_rom(&pairs, &layout, &header)?)
}

fn assemble_symbolic_program_rom(
    section: Section,
    extra: Vec<RomExtraSection>,
    title: &str,
    rom_size: RomSize,
) -> Result<Vec<u8>, BringupRomBuildError> {
    let (rom, _) = assemble_symbolic_program_rom_inner(
        section,
        extra,
        title,
        rom_size,
        &[],
        Vec::new(),
        Vec::new(),
    )?;
    Ok(rom)
}

fn assemble_symbolic_program_rom_with_symbols(
    section: Section,
    extra: Vec<RomExtraSection>,
    title: &str,
    rom_size: RomSize,
    symbols: &[SymbolName],
    support_sections: Vec<Section>,
    extra_pins: Vec<PinnedPlacement>,
) -> Result<(Vec<u8>, Vec<u16>), BringupRomBuildError> {
    assemble_symbolic_program_rom_inner(
        section,
        extra,
        title,
        rom_size,
        symbols,
        support_sections,
        extra_pins,
    )
}

fn assemble_symbolic_program_rom_inner(
    section: Section,
    extra: Vec<RomExtraSection>,
    title: &str,
    rom_size: RomSize,
    requested_symbols: &[SymbolName],
    support_sections: Vec<Section>,
    extra_pins: Vec<PinnedPlacement>,
) -> Result<(Vec<u8>, Vec<u16>), BringupRomBuildError> {
    let lowerer = BankingPreLayoutLowering::default().with_residency(
        section.id(),
        gbf_runtime::banking::SectionResidency::FixedRom0,
    );
    let mut sections_to_lower = Vec::with_capacity(1 + support_sections.len());
    sections_to_lower.push(section);
    sections_to_lower.extend(support_sections);
    let lowered = lower_pre_layout_ops(sections_to_lower, &lowerer, &SymbolTable::new())
        .map_err(|error| BringupRomBuildError::pipeline("pre-layout lowering", error))?;
    let mut pins = Vec::with_capacity(1 + extra_pins.len());
    pins.push(PinnedPlacement {
        section_id: FB1_L0_ROM_SECTION_ID,
        bank: BankIndex::Rom(0),
        cpu_start: gbf_asm::rom::ENTRY_POINT,
    });
    pins.extend(extra_pins);
    let layout = layout_into_banks(&lowered, PlacementProfile::PackedExperts, &pins)
        .map_err(|error| BringupRomBuildError::pipeline("layout", error))?;
    let relaxed = relax_and_legalize(&lowered, &layout)
        .map_err(|error| BringupRomBuildError::pipeline("relaxation", error))?;
    let requested_symbol_addrs = requested_symbols
        .iter()
        .map(|symbol| symbol_cpu_addr(&relaxed.layout, &relaxed.symbols, symbol))
        .collect::<Result<Vec<_>, _>>()?;

    let mut pairs = Vec::with_capacity(1 + extra.len());
    let mut sections = relaxed.layout.sections.clone();
    for section in &relaxed.sections {
        let placed = relaxed
            .layout
            .sections
            .iter()
            .find(|candidate| candidate.id == section.id)
            .expect("relaxed section has placement")
            .clone();
        let encoded = encode_section(section, &placed)?;
        pairs.push((encoded, placed));
    }

    for section in extra {
        let encoded = EncodedSection {
            id: section.id,
            bytes: section.bytes,
            item_spans: Vec::new(),
        };
        let placed = PlacedSection {
            id: section.id,
            space: AddressSpace::RomX,
            bank: BankIndex::Rom(section.bank),
            cpu_start: section.cpu_start,
            final_size: u16::try_from(encoded.bytes.len()).expect("extra section fits u16"),
            estimated_size: u16::try_from(encoded.bytes.len()).expect("extra section fits u16"),
            alignment_padding: Default::default(),
        };
        sections.push(placed.clone());
        pairs.push((encoded, placed));
    }

    let layout = LayoutPlan {
        sections,
        bank_count: rom_size.bank_count(),
        free_bytes_per_bank: Default::default(),
        reserved_ranges: relaxed.layout.reserved_ranges,
    };
    let mut header = CartridgeHeader::new(title)?;
    header.rom_size = rom_size;
    Ok((
        assemble_rom(&pairs, &layout, &header)?,
        requested_symbol_addrs,
    ))
}

fn symbol_cpu_addr(
    layout: &LayoutPlan,
    symbols: &SymbolTable,
    symbol: &SymbolName,
) -> Result<u16, BringupRomBuildError> {
    let address = symbols.resolve(symbol).ok_or_else(|| {
        BringupRomBuildError::pipeline("symbol resolution", format!("missing symbol {symbol}"))
    })?;
    let placed = layout
        .sections
        .iter()
        .find(|section| section.id == address.section)
        .ok_or_else(|| {
            BringupRomBuildError::pipeline(
                "symbol resolution",
                format!(
                    "symbol {symbol} points at unplaced section {}",
                    address.section.get()
                ),
            )
        })?;
    let cpu_addr = u32::from(placed.cpu_start) + address.offset;
    u16::try_from(cpu_addr).map_err(|_| {
        BringupRomBuildError::pipeline(
            "symbol resolution",
            format!("symbol {symbol} resolves outside CPU address space: {cpu_addr}"),
        )
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BringupRomWindowPlan {
    pub kernel: RomResidency,
    pub multiply_table: RomResidency,
    pub operand_a: RomResidency,
    pub operand_b: RomResidency,
    pub bank0_used_bytes: u32,
    pub bank0_free_bytes: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RomResidency {
    PermanentBank0 {
        offset: u16,
        length: u16,
    },
    SwitchableRom {
        bank: RomBankId,
        offset: u16,
        length: u16,
    },
}

pub fn plan_rom_window(
    request: &ComputeBringupRequest,
) -> Result<BringupRomWindowPlan, ComputeBringupRequestError> {
    request.validate()?;
    let operand_len = u16::try_from(request.matrix_dim.operand_bytes()).expect("operand fits u16");
    let (operand_a, operand_b) = match request.operand_layout {
        OperandLayout::WramSmoke => (
            RomResidency::SwitchableRom {
                bank: RomBankId::new(1).expect("bank 1"),
                offset: 0,
                length: operand_len,
            },
            RomResidency::SwitchableRom {
                bank: RomBankId::new(2).expect("bank 2"),
                offset: 0,
                length: operand_len,
            },
        ),
        OperandLayout::DistinctRomBanks { a, b } => (
            RomResidency::SwitchableRom {
                bank: a.bank,
                offset: a.offset.get(),
                length: operand_len,
            },
            RomResidency::SwitchableRom {
                bank: b.bank,
                offset: b.offset.get(),
                length: operand_len,
            },
        ),
    };
    let runtime_used: u32 = gbf_runtime::runtime_nucleus_section_sizes()
        .iter()
        .map(|section| section.bytes)
        .sum();
    let kernel_len = 512_u16;
    let bank0_used_bytes = runtime_used + u32::from(kernel_len);
    let bank0_free_bytes = gbf_hw::memory::BANK0_SIZE_BYTES - bank0_used_bytes;
    Ok(BringupRomWindowPlan {
        kernel: RomResidency::PermanentBank0 {
            offset: 0x2000,
            length: kernel_len,
        },
        multiply_table: RomResidency::PermanentBank0 {
            offset: 0x2200,
            length: 0,
        },
        operand_a,
        operand_b,
        bank0_used_bytes,
        bank0_free_bytes,
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BringupStoragePlan {
    pub accumulator: StorageBinding,
    pub a_panel: StorageBinding,
    pub b_panel: StorageBinding,
    pub output: OutputBinding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageClass {
    WramScratch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StorageBinding {
    pub class: StorageClass,
    pub offset: u16,
    pub length: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OutputBinding {
    HarnessTileStream {
        source: WramArenaSlot,
        tile_bytes: u16,
        order: TileOrder,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WramArenaSlot {
    pub offset: u16,
    pub length: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileOrder {
    MtMajorNtMinorRowMajorI32Le,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BringupArenaPlan {
    pub accumulator_tile: WramArenaSlot,
    pub a_panel: WramArenaSlot,
    pub b_panel: WramArenaSlot,
}

impl BringupArenaPlan {
    pub fn validate(&self) -> Result<(), ArenaPlanError> {
        if !self.accumulator_tile.offset.is_multiple_of(4) {
            return Err(ArenaPlanError::AccumulatorUnaligned {
                offset: self.accumulator_tile.offset,
            });
        }
        let slots = [self.accumulator_tile, self.a_panel, self.b_panel];
        for slot in slots {
            let end = u32::from(slot.offset) + u32::from(slot.length);
            if end > gbf_hw::memory::WRAM_SIZE_BYTES {
                return Err(ArenaPlanError::WramOverflow {
                    offset: slot.offset,
                    length: slot.length,
                });
            }
        }
        for (idx, left) in slots.iter().enumerate() {
            for right in slots.iter().skip(idx + 1) {
                if ranges_overlap(*left, *right) {
                    return Err(ArenaPlanError::Overlap {
                        left: *left,
                        right: *right,
                    });
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArenaPlanError {
    AccumulatorUnaligned {
        offset: u16,
    },
    WramOverflow {
        offset: u16,
        length: u16,
    },
    Overlap {
        left: WramArenaSlot,
        right: WramArenaSlot,
    },
}

impl fmt::Display for ArenaPlanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AccumulatorUnaligned { offset } => {
                write!(f, "accumulator offset {offset} is not 4-byte aligned")
            }
            Self::WramOverflow { offset, length } => {
                write!(f, "WRAM slot {offset}..+{length} exceeds WRAM")
            }
            Self::Overlap { left, right } => {
                write!(f, "WRAM slots {left:?} and {right:?} overlap")
            }
        }
    }
}

impl std::error::Error for ArenaPlanError {}

#[must_use]
pub fn plan_arena() -> BringupArenaPlan {
    BringupArenaPlan {
        accumulator_tile: WramArenaSlot {
            offset: 0x0000,
            length: ACCUMULATOR_TILE_BYTES,
        },
        a_panel: WramArenaSlot {
            offset: ACCUMULATOR_TILE_BYTES,
            length: PANEL_TILE_BYTES,
        },
        b_panel: WramArenaSlot {
            offset: ACCUMULATOR_TILE_BYTES + PANEL_TILE_BYTES,
            length: PANEL_TILE_BYTES,
        },
    }
}

#[must_use]
pub fn plan_storage(arena: BringupArenaPlan) -> BringupStoragePlan {
    BringupStoragePlan {
        accumulator: StorageBinding {
            class: StorageClass::WramScratch,
            offset: arena.accumulator_tile.offset,
            length: arena.accumulator_tile.length,
        },
        a_panel: StorageBinding {
            class: StorageClass::WramScratch,
            offset: arena.a_panel.offset,
            length: arena.a_panel.length,
        },
        b_panel: StorageBinding {
            class: StorageClass::WramScratch,
            offset: arena.b_panel.offset,
            length: arena.b_panel.length,
        },
        output: OutputBinding::HarnessTileStream {
            source: arena.accumulator_tile,
            tile_bytes: ACCUMULATOR_TILE_BYTES,
            order: TileOrder::MtMajorNtMinorRowMajorI32Le,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileCoord {
    pub tile_index: u16,
    pub mt: u16,
    pub nt: u16,
}

#[must_use]
pub fn tile_schedule(dim: SquareDim) -> Vec<TileCoord> {
    let tiles = dim.tiles_per_axis();
    let mut out = Vec::with_capacity(usize::from(tiles * tiles));
    let mut tile_index = 0_u16;
    for mt in 0..tiles {
        for nt in 0..tiles {
            out.push(TileCoord { tile_index, mt, nt });
            tile_index += 1;
        }
    }
    out
}

pub fn lower_request_to_ir(
    request: &ComputeBringupRequest,
) -> Result<InferGraph, ComputeBringupRequestError> {
    request.validate()?;
    let input_storage = match request.operand_layout {
        OperandLayout::WramSmoke => TensorStorage::WramSmoke,
        OperandLayout::DistinctRomBanks { .. } => TensorStorage::SwitchableRom,
    };
    let graph = InferGraph::f_b1_single_matmul(MatmulI8Node {
        a: TensorBinding::read_only("A", input_storage),
        b: TensorBinding::read_only("B", input_storage),
        out: TensorBinding::writable("C", TensorStorage::HarnessStreamedOutput),
        dim: request.matrix_dim,
        tile_size: request.tile_size.into(),
    })
    .expect("validated F-B1 request builds valid IR");
    emit_fb1_event(&FB1TraceEvent::LowerToIrComplete {
        ir_hash: format!("sha256:{}", hash_json(&graph)),
    });
    Ok(graph)
}

pub fn lower_request_to_asmir(
    request: &ComputeBringupRequest,
) -> Result<Section, ComputeBringupRequestError> {
    request.validate()?;
    let mut builder = Builder::new_with_id(
        FB1_KERNEL_SECTION_ID,
        SectionRole::Bank0Nucleus,
        SymbolName::runtime("f_b1", "matmul_kernel").expect("static symbol"),
    )
    .with_section_privilege(SectionPrivilege::normal());
    builder.emit(Instr::Nop);
    if let OperandLayout::DistinctRomBanks { a, b } = request.operand_layout {
        emit_operand_copy_lease(&mut builder, 0, a.bank.get());
        emit_operand_copy_lease(&mut builder, 1, b.bank.get());
    }
    builder.yield_op(YieldKind::Cooperative);
    builder.emit(Instr::Nop);
    let section = builder.finish().with_size_hint_bytes(512);
    emit_fb1_event(&FB1TraceEvent::LowerToAsmIrComplete {
        asmir_hash: format!("sha256:{}", hash_json(&section)),
    });
    Ok(section)
}

pub fn validate_no_raw_mbc_writes(section: &Section) -> Result<(), BringupValidationError> {
    let has_raw = section.iter_items().into_iter().any(|item| {
        item.machine_effect()
            .is_some_and(|effect| effect.kind() == MachineEffectKind::StoreToMbcRegister)
    });
    if has_raw {
        Err(BringupValidationError::RawMbcWrite {
            section: section.id(),
        })
    } else {
        Ok(())
    }
}

pub fn validate_uses_banklease_ops(section: &Section) -> Result<(), BringupValidationError> {
    let lease_count = section
        .pre_layout_ops()
        .iter()
        .filter(|op| matches!(op.data, PreLayoutOp::BankLease(_)))
        .count();
    let release_count = section
        .pre_layout_ops()
        .iter()
        .filter(|op| matches!(op.data, PreLayoutOp::BankRelease { .. }))
        .count();
    if lease_count == 0 || release_count == 0 {
        return Err(BringupValidationError::MissingBankLeaseOps {
            section: section.id(),
        });
    }
    if lease_count != release_count {
        return Err(BringupValidationError::UnbalancedBankLeases {
            section: section.id(),
            acquired: lease_count,
            released: release_count,
        });
    }
    validate_no_yield_while_lease_active(section)
}

pub fn validate_no_yield_while_lease_active(
    section: &Section,
) -> Result<(), BringupValidationError> {
    let mut active = BTreeSet::new();
    let mut acquired = BTreeSet::new();
    let mut released = BTreeSet::new();
    for op in section.pre_layout_ops() {
        match &op.data {
            PreLayoutOp::BankLease(spec) => {
                let lease_id = spec.lease_id();
                if !acquired.insert(lease_id) || !active.insert(lease_id) {
                    return Err(BringupValidationError::DuplicateBankLease {
                        section: section.id(),
                        lease_id: lease_id.get(),
                    });
                }
            }
            PreLayoutOp::BankRelease { lease_id, .. } => {
                if !active.remove(lease_id) {
                    return Err(BringupValidationError::UnknownBankRelease {
                        section: section.id(),
                        lease_id: lease_id.get(),
                    });
                }
                if !released.insert(*lease_id) {
                    return Err(BringupValidationError::DuplicateBankRelease {
                        section: section.id(),
                        lease_id: lease_id.get(),
                    });
                }
            }
            PreLayoutOp::Yield { .. } if !active.is_empty() => {
                return Err(BringupValidationError::YieldWithActiveLease {
                    section: section.id(),
                    active: active.iter().map(|lease| lease.get()).collect(),
                });
            }
            _ => {}
        }
    }
    if !active.is_empty() {
        return Err(BringupValidationError::UnreleasedBankLeases {
            section: section.id(),
            active: active.iter().map(|lease| lease.get()).collect(),
        });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BringupValidationError {
    RawMbcWrite {
        section: SectionId,
    },
    MissingBankLeaseOps {
        section: SectionId,
    },
    UnbalancedBankLeases {
        section: SectionId,
        acquired: usize,
        released: usize,
    },
    DuplicateBankLease {
        section: SectionId,
        lease_id: u32,
    },
    DuplicateBankRelease {
        section: SectionId,
        lease_id: u32,
    },
    UnknownBankRelease {
        section: SectionId,
        lease_id: u32,
    },
    UnreleasedBankLeases {
        section: SectionId,
        active: Vec<u32>,
    },
    YieldWithActiveLease {
        section: SectionId,
        active: Vec<u32>,
    },
}

impl fmt::Display for BringupValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RawMbcWrite { section } => {
                write!(f, "section {} contains raw MBC writes", section.get())
            }
            Self::MissingBankLeaseOps { section } => {
                write!(
                    f,
                    "section {} lacks BankLease/BankRelease ops",
                    section.get()
                )
            }
            Self::UnbalancedBankLeases {
                section,
                acquired,
                released,
            } => {
                write!(
                    f,
                    "section {} has unbalanced BankLease ops: acquired {acquired}, released {released}",
                    section.get()
                )
            }
            Self::DuplicateBankLease { section, lease_id } => {
                write!(
                    f,
                    "section {} reuses active BankLease id {lease_id}",
                    section.get()
                )
            }
            Self::DuplicateBankRelease { section, lease_id } => {
                write!(
                    f,
                    "section {} releases BankLease id {lease_id} more than once",
                    section.get()
                )
            }
            Self::UnknownBankRelease { section, lease_id } => {
                write!(
                    f,
                    "section {} releases unknown BankLease id {lease_id}",
                    section.get()
                )
            }
            Self::UnreleasedBankLeases { section, active } => {
                write!(
                    f,
                    "section {} exits with active BankLease ids {active:?}",
                    section.get()
                )
            }
            Self::YieldWithActiveLease { section, active } => {
                write!(
                    f,
                    "section {} yields with active leases {active:?}",
                    section.get()
                )
            }
        }
    }
}

impl std::error::Error for BringupValidationError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BringupRun {
    pub dim: SquareDim,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rom_sha256: Option<String>,
    pub output_i32: Vec<i32>,
    pub output_tiles: Vec<TileDump>,
    pub metrics: BringupRunMetrics,
    pub frame_events: Vec<FrameEventEnvelope>,
}

impl BringupRun {
    #[must_use]
    pub fn output_bytes_le(&self) -> Vec<u8> {
        i32s_to_le_bytes(&self.output_i32)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TileDump {
    pub tile_index: u16,
    pub mt: u16,
    pub nt: u16,
    pub source_wram_addr: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BringupRunMetrics {
    pub products: u64,
    pub cycles_per_full_matmul_mcycles: u64,
    pub output_tiles: u32,
    pub k_tiles_per_output_tile: u32,
    pub operand_panel_copies: u32,
    pub operand_panel_bytes_copied: u32,
    pub full_output_bytes: u32,
    pub bank_lease_acquire_count: u32,
    pub bank_lease_release_count: u32,
    pub bank_lease_balance: i32,
    pub max_active_bank_leases: u16,
    pub yield_while_bank_lease_active_count: u32,
    pub harness_pause_while_bank_lease_active_count: u32,
    pub widget_update_count: u32,
    pub scheduler_service_count: u32,
    pub frame_service_misses: u32,
    pub max_no_progress_frames: u32,
    pub max_unyielded_compute_mcycles: u32,
    pub max_bank_lease_hold_mcycles: u32,
    pub cycles_per_product_sample_count: u32,
}

impl BringupRunMetrics {
    #[must_use]
    pub fn structural(dim: SquareDim) -> Self {
        let n = u64::from(dim.n());
        let output_tiles = u32::from(dim.tiles_per_axis()) * u32::from(dim.tiles_per_axis());
        let k_tiles = u32::from(dim.tiles_per_axis());
        let panel_copies = output_tiles * k_tiles * 2;
        Self {
            products: n * n * n,
            cycles_per_full_matmul_mcycles: n * n * n * 4,
            output_tiles,
            k_tiles_per_output_tile: k_tiles,
            operand_panel_copies: panel_copies,
            operand_panel_bytes_copied: panel_copies * u32::from(PANEL_TILE_BYTES),
            full_output_bytes: u32::try_from(dim.output_bytes_i32()).expect("fits u32"),
            bank_lease_acquire_count: panel_copies,
            bank_lease_release_count: panel_copies,
            bank_lease_balance: 0,
            max_active_bank_leases: 1,
            yield_while_bank_lease_active_count: 0,
            harness_pause_while_bank_lease_active_count: 0,
            widget_update_count: 0,
            scheduler_service_count: 0,
            frame_service_misses: 0,
            max_no_progress_frames: 1,
            max_unyielded_compute_mcycles: 320,
            max_bank_lease_hold_mcycles: 128,
            cycles_per_product_sample_count: u32::try_from(n).expect("fits u32"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrameEventEnvelope {
    pub seq: u64,
    pub event: FrameEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "PascalCase")]
pub enum FrameEvent {
    VBlankFired {
        frame: u32,
        mcycle_since_boot: u64,
    },
    WidgetTickDispatched {
        frame: u32,
        mcycle_since_boot: u64,
    },
    SchedulerServicedFrame {
        frame: u32,
        mcycle_since_boot: u64,
    },
    YieldReturnedToScheduler {
        frame: u32,
        mcycle_since_boot: u64,
        remaining_frame_mcycles_i32: i32,
        completed_quantum_products: u16,
        compute_progress_epoch: u32,
    },
    ComputeProgressEpochAdvanced {
        frame: u32,
        mcycle_since_boot: u64,
        compute_progress_epoch: u32,
    },
}

pub fn run_bringup_model(
    request: &ComputeBringupRequest,
) -> Result<BringupRun, ComputeBringupRequestError> {
    request.validate()?;
    let dim = request.matrix_dim;
    let n = usize::from(dim.n());
    let a_bytes = materialize_operand_fixture(request.operand_fixture, OperandKind::A, dim);
    let b_bytes = materialize_operand_fixture(request.operand_fixture, OperandKind::B, dim);
    let a: Vec<i8> = a_bytes.iter().copied().map(|value| value as i8).collect();
    let b: Vec<i8> = b_bytes.iter().copied().map(|value| value as i8).collect();
    let mut out = vec![0_i32; dim.elem_count()];
    for i in 0..n {
        for j in 0..n {
            let mut acc = 0_i32;
            for k in 0..n {
                acc += quarter_square_mul_i8(a[i * n + k], b[k * n + j]);
            }
            out[i * n + j] = acc;
        }
    }

    let mut output_tiles = Vec::new();
    for coord in tile_schedule(dim) {
        output_tiles.push(TileDump {
            tile_index: coord.tile_index,
            mt: coord.mt,
            nt: coord.nt,
            source_wram_addr: 0xC000,
            bytes: tile_bytes(&out, dim, coord.mt, coord.nt),
        });
    }

    let mut metrics = BringupRunMetrics::structural(dim);
    let frame_count = metrics.output_tiles.max(2) + 2;
    metrics.widget_update_count = frame_count;
    metrics.scheduler_service_count = frame_count;
    let frame_events = synthetic_frame_events(frame_count, request.yield_quantum);
    Ok(BringupRun {
        dim,
        rom_sha256: None,
        output_i32: out,
        output_tiles,
        metrics,
        frame_events,
    })
}

pub fn reassemble_tiles(dim: SquareDim, tiles: &[TileDump]) -> Vec<i32> {
    let n = usize::from(dim.n());
    let mut out = vec![0_i32; dim.elem_count()];
    for tile in tiles {
        let row_base = usize::from(tile.mt) * usize::from(TILE_EDGE);
        let col_base = usize::from(tile.nt) * usize::from(TILE_EDGE);
        for mm in 0..usize::from(TILE_EDGE) {
            for nn in 0..usize::from(TILE_EDGE) {
                let byte_offset = (mm * usize::from(TILE_EDGE) + nn) * 4;
                let value = i32::from_le_bytes(
                    tile.bytes[byte_offset..byte_offset + 4]
                        .try_into()
                        .expect("tile i32 bytes"),
                );
                out[(row_base + mm) * n + col_base + nn] = value;
            }
        }
    }
    out
}

pub fn validate_structural_counts(
    metrics: &BringupRunMetrics,
    dim: SquareDim,
) -> Result<(), String> {
    let expected = BringupRunMetrics::structural(dim);
    if metrics.products != expected.products {
        return Err(format!(
            "products {} != {}",
            metrics.products, expected.products
        ));
    }
    if metrics.output_tiles != expected.output_tiles {
        return Err(format!(
            "output_tiles {} != {}",
            metrics.output_tiles, expected.output_tiles
        ));
    }
    if metrics.k_tiles_per_output_tile != expected.k_tiles_per_output_tile {
        return Err(format!(
            "k_tiles_per_output_tile {} != {}",
            metrics.k_tiles_per_output_tile, expected.k_tiles_per_output_tile
        ));
    }
    if metrics.operand_panel_copies != expected.operand_panel_copies {
        return Err(format!(
            "operand_panel_copies {} != {}",
            metrics.operand_panel_copies, expected.operand_panel_copies
        ));
    }
    if metrics.operand_panel_bytes_copied != expected.operand_panel_bytes_copied {
        return Err(format!(
            "operand_panel_bytes_copied {} != {}",
            metrics.operand_panel_bytes_copied, expected.operand_panel_bytes_copied
        ));
    }
    if metrics.full_output_bytes != expected.full_output_bytes {
        return Err(format!(
            "full_output_bytes {} != {}",
            metrics.full_output_bytes, expected.full_output_bytes
        ));
    }
    Ok(())
}

fn deterministic_operand(dim: SquareDim, f: impl Fn(i32, i32) -> i32) -> Vec<u8> {
    let n = usize::from(dim.n());
    let mut out = Vec::with_capacity(dim.elem_count());
    for i in 0..n {
        for j in 0..n {
            let raw = f(i as i32, j as i32).rem_euclid(256) - 128;
            let value = i8::try_from(raw).expect("affine fixture stays in i8 range");
            out.push(value as u8);
        }
    }
    out
}

fn validate_placement(
    operand: &str,
    placement: BankedOperandPlacement,
    len: u32,
    dim: SquareDim,
) -> Result<(), ComputeBringupRequestError> {
    let offset = u32::from(placement.offset.get());
    if offset + len > ROM_BANK_SIZE_BYTES {
        return Err(ComputeBringupRequestError::BadOperandOffset {
            operand: operand.to_owned(),
            offset: placement.offset.get(),
            len,
        });
    }
    if dim.n() == 128 && placement.offset.get() != 0 {
        return Err(ComputeBringupRequestError::N128RequiresZeroOffset {
            operand: operand.to_owned(),
            offset: placement.offset.get(),
        });
    }
    Ok(())
}

fn emit_operand_copy_lease(builder: &mut Builder, lease_id: u32, bank: u16) {
    let lease_id = LeaseId::new(lease_id);
    let spec = BankLeaseSpec::new(
        lease_id,
        LeaseGeneration(lease_id.get() + 1),
        MbcBankClass::Rom,
        bank,
        LeaseLifetime::Slice,
    )
    .expect("validated bank fits lease spec");
    builder.bank_lease(spec);
    builder.emit(Instr::Nop);
    builder.bank_release_to(lease_id, BankReleaseDisposition::RomBank1);
}

fn ranges_overlap(left: WramArenaSlot, right: WramArenaSlot) -> bool {
    let left_start = u32::from(left.offset);
    let left_end = left_start + u32::from(left.length);
    let right_start = u32::from(right.offset);
    let right_end = right_start + u32::from(right.length);
    left_start < right_end && right_start < left_end
}

fn tile_bytes(out: &[i32], dim: SquareDim, mt: u16, nt: u16) -> Vec<u8> {
    let n = usize::from(dim.n());
    let row_base = usize::from(mt) * usize::from(TILE_EDGE);
    let col_base = usize::from(nt) * usize::from(TILE_EDGE);
    let mut bytes = Vec::with_capacity(ACCUMULATOR_TILE_BYTES as usize);
    for mm in 0..usize::from(TILE_EDGE) {
        for nn in 0..usize::from(TILE_EDGE) {
            bytes.extend_from_slice(&out[(row_base + mm) * n + col_base + nn].to_le_bytes());
        }
    }
    bytes
}

fn i32s_to_le_bytes(values: &[i32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn synthetic_frame_events(frame_count: u32, quantum: YieldQuantum) -> Vec<FrameEventEnvelope> {
    let mut seq = 0_u64;
    let mut events = Vec::with_capacity(frame_count as usize * 5);
    for frame in 0..frame_count {
        let base = u64::from(frame) * u64::from(gbf_hw::timing::FRAME_M_CYCLES);
        push_frame_event(
            &mut events,
            &mut seq,
            FrameEvent::VBlankFired {
                frame,
                mcycle_since_boot: base,
            },
        );
        push_frame_event(
            &mut events,
            &mut seq,
            FrameEvent::WidgetTickDispatched {
                frame,
                mcycle_since_boot: base + 32,
            },
        );
        push_frame_event(
            &mut events,
            &mut seq,
            FrameEvent::SchedulerServicedFrame {
                frame,
                mcycle_since_boot: base + 64,
            },
        );
        push_frame_event(
            &mut events,
            &mut seq,
            FrameEvent::YieldReturnedToScheduler {
                frame,
                mcycle_since_boot: base + 128,
                remaining_frame_mcycles_i32: gbf_hw::timing::FRAME_M_CYCLES as i32 - 128,
                completed_quantum_products: quantum.products_per_quantum(),
                compute_progress_epoch: frame + 1,
            },
        );
        push_frame_event(
            &mut events,
            &mut seq,
            FrameEvent::ComputeProgressEpochAdvanced {
                frame,
                mcycle_since_boot: base + 129,
                compute_progress_epoch: frame + 1,
            },
        );
    }
    events
}

fn push_frame_event(events: &mut Vec<FrameEventEnvelope>, seq: &mut u64, event: FrameEvent) {
    events.push(FrameEventEnvelope { seq: *seq, event });
    *seq += 1;
}

fn hash_json(value: &impl Serialize) -> Hash256 {
    let bytes = serde_json::to_vec(value).expect("serializes");
    hash_domain(b"gbf-codegen/f-b1/json", &bytes)
}

fn hash_domain(domain: &[u8], bytes: &[u8]) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    Hash256::from_bytes(hasher.finalize().into())
}

#[cfg(test)]
pub mod test_fixtures {
    use super::*;
    use gbf_asm::builder::BuilderError;
    use gbf_asm::isa::{DirectAddr, Reg8};
    use gbf_asm::section::SectionPrivilege;

    #[must_use]
    pub fn well_typed_panel_copy() -> Section {
        lower_request_to_asmir(&fixture_request()).expect("section")
    }

    fn fixture_request() -> ComputeBringupRequest {
        ComputeBringupRequest {
            matrix_dim: SquareDim::new(64).expect("valid"),
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..ComputeBringupRequest::l1_wram_smoke()
        }
    }

    #[must_use]
    pub fn violation_raw_mbc_write() -> Section {
        let mut builder = Builder::new_with_id(
            SectionId::new(0xB1FE),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("f_b1_fixture", "raw_mbc").expect("symbol"),
        )
        .with_section_privilege(SectionPrivilege::privileged());
        builder.emit(Instr::Ld8RegFromImm {
            dst: Reg8::A,
            imm: 1,
        });
        builder.emit(Instr::LdDirectFromA {
            addr: DirectAddr::new(0x2000).expect("mbc register"),
        });
        builder.finish()
    }

    pub fn violation_yield_under_lease_attempt() -> BuilderError {
        let mut builder = Builder::new_with_id(
            SectionId::new(0xB1FD),
            SectionRole::Bank0Nucleus,
            SymbolName::runtime("f_b1_fixture", "yield_under_lease").expect("symbol"),
        );
        let lease = BankLeaseSpec::new(
            LeaseId::new(9),
            LeaseGeneration(9),
            MbcBankClass::Rom,
            3,
            LeaseLifetime::Slice,
        )
        .expect("lease");
        builder.bank_lease(lease);
        builder
            .try_yield_op(YieldKind::Cooperative)
            .expect_err("builder rejects yield under lease")
    }

    #[test]
    fn synthetic_asmir_fixtures_load() {
        assert_eq!(well_typed_panel_copy().id(), FB1_KERNEL_SECTION_ID);
        assert_eq!(
            violation_raw_mbc_write().role(),
            gbf_asm::section::SectionRole::Bank0Nucleus
        );
    }

    #[test]
    fn all_violation_fixtures_actually_violate() {
        validate_no_raw_mbc_writes(&violation_raw_mbc_write()).expect_err("raw mbc rejected");
        assert!(matches!(
            violation_yield_under_lease_attempt(),
            BuilderError::YieldWithActiveLease { .. }
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_distinct(n: u16) -> ComputeBringupRequest {
        ComputeBringupRequest {
            matrix_dim: SquareDim::new(n).expect("valid"),
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..ComputeBringupRequest::l1_wram_smoke()
        }
    }

    #[test]
    fn compute_bringup_request_accepts_valid() {
        valid_distinct(128).validate().expect("valid");
    }

    #[test]
    fn f_b1_request_validation_accepts_l1_smoke_layout() {
        ComputeBringupRequest::l1_wram_smoke()
            .validate()
            .expect("valid smoke");
    }

    #[test]
    fn f_b1_request_validation_rejects_smoke_layout_for_n_gt_16() {
        let request = ComputeBringupRequest {
            matrix_dim: SquareDim::new(32).expect("valid"),
            ..ComputeBringupRequest::l1_wram_smoke()
        };
        assert_eq!(
            request.validate(),
            Err(ComputeBringupRequestError::SmokeLayoutRequiresN16 { found: 32 })
        );
    }

    #[test]
    fn compute_bringup_request_rejects_same_bank() {
        let request = ComputeBringupRequest {
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..valid_distinct(64)
        };
        assert_eq!(
            request.validate(),
            Err(ComputeBringupRequestError::SameBank { bank: 1 })
        );
    }

    #[test]
    fn compute_bringup_request_rejects_bank0_operand() {
        assert_eq!(RomBankId::new(0), Err(ComputeBringupRequestError::BankZero));
    }

    #[test]
    fn compute_bringup_request_rejects_bad_operand_offset() {
        let request = ComputeBringupRequest {
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(1).expect("bank"),
                    offset: RomBankOffset::new(16_000),
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..valid_distinct(64)
        };
        assert!(matches!(
            request.validate(),
            Err(ComputeBringupRequestError::BadOperandOffset { operand, .. }) if operand == "A"
        ));
    }

    #[test]
    fn compute_bringup_request_rejects_bad_tile() {
        let request = ComputeBringupRequest {
            tile_size: TileSize { m: 8, n: 16, k: 16 },
            ..valid_distinct(64)
        };
        assert!(matches!(
            request.validate(),
            Err(ComputeBringupRequestError::BadTile { .. })
        ));
    }

    #[test]
    fn compute_bringup_request_hash_includes_fixture_and_layout() {
        let base = valid_distinct(64);
        let base_hash = base.hash().expect("hash");
        let shifted = ComputeBringupRequest {
            operand_layout: OperandLayout::DistinctRomBanks {
                a: BankedOperandPlacement {
                    bank: RomBankId::new(3).expect("bank"),
                    offset: RomBankOffset::new(16),
                },
                b: BankedOperandPlacement {
                    bank: RomBankId::new(2).expect("bank"),
                    offset: RomBankOffset::ZERO,
                },
            },
            ..base.clone()
        };
        assert_ne!(base_hash, shifted.hash().expect("hash"));
    }

    #[test]
    fn operand_fixture_matches_verify_for_review_sizes() {
        for n in [32, 64, 96, 128] {
            let dim = SquareDim::new(n).expect("valid");
            assert_eq!(
                materialize_operand_fixture(
                    OperandFixtureSpec::DeterministicAffineV1,
                    OperandKind::A,
                    dim
                ),
                gbf_verify::matmul::deterministic_operand_bytes_a(dim)
            );
            assert_eq!(
                materialize_operand_fixture(
                    OperandFixtureSpec::DeterministicAffineV1,
                    OperandKind::B,
                    dim
                ),
                gbf_verify::matmul::deterministic_operand_bytes_b(dim)
            );
        }
    }

    #[test]
    fn quarter_square_table_shape() {
        let table = quarter_square_table_i16();
        assert_eq!(table.len(), QUARTER_SQUARE_LEN);
        assert_eq!(
            quarter_square_table_bytes_le().len(),
            QUARTER_SQUARE_BYTES as usize
        );
        assert_eq!(
            quarter_square_table_split_bytes().len(),
            QUARTER_SQUARE_BYTES as usize
        );
        assert_eq!(
            &quarter_square_table_split_bytes()[..4],
            &[
                table[0].to_le_bytes()[0],
                table[1].to_le_bytes()[0],
                table[2].to_le_bytes()[0],
                table[3].to_le_bytes()[0]
            ]
        );
        assert_eq!(
            &quarter_square_table_split_bytes()[512..516],
            &[
                table[0].to_le_bytes()[1],
                table[1].to_le_bytes()[1],
                table[2].to_le_bytes()[1],
                table[3].to_le_bytes()[1]
            ]
        );
        assert_eq!(table[0], 16_384);
        assert!(table.iter().all(|value| *value >= 0));
    }

    #[test]
    fn quarter_square_table_matches_verify() {
        assert_eq!(
            quarter_square_table_i16(),
            gbf_verify::matmul::quarter_square_table_reference_i16()
        );
    }

    #[test]
    fn quarter_square_mul_exhaustive_i8() {
        for a in i8::MIN..=i8::MAX {
            for b in i8::MIN..=i8::MAX {
                assert_eq!(quarter_square_mul_i8(a, b), i32::from(a) * i32::from(b));
            }
        }
    }

    #[test]
    fn f_b1_rom_window_plan_distinct_banks() {
        let plan = plan_rom_window(&valid_distinct(64)).expect("plan");
        assert!(matches!(
            plan.operand_a,
            RomResidency::SwitchableRom { bank, .. } if bank.get() == 1
        ));
        assert!(matches!(
            plan.operand_b,
            RomResidency::SwitchableRom { bank, .. } if bank.get() == 2
        ));
    }

    #[test]
    fn rom_window_plan_is_deterministic() {
        assert_eq!(
            plan_rom_window(&valid_distinct(64)).expect("plan"),
            plan_rom_window(&valid_distinct(64)).expect("plan")
        );
    }

    #[test]
    fn rom_window_plan_bank0_resources() {
        let plan = plan_rom_window(&valid_distinct(128)).expect("plan");
        assert!(matches!(plan.kernel, RomResidency::PermanentBank0 { .. }));
        assert!(matches!(
            plan.multiply_table,
            RomResidency::PermanentBank0 { length: 0, .. }
        ));
        assert!(plan.bank0_free_bytes > 0);
    }

    #[test]
    fn f_b1_l2_lowering_uses_banklease_ops() {
        let section = lower_request_to_asmir(&valid_distinct(64)).expect("section");
        validate_uses_banklease_ops(&section).expect("uses leases");
    }

    #[test]
    fn f_b1_banklease_validator_rejects_leaked_lease() {
        let section = lower_request_to_asmir(&valid_distinct(64)).expect("section");
        let mut json = serde_json::to_value(&section).expect("section serializes");
        let ops = json["pre_layout_ops"]
            .as_array_mut()
            .expect("pre-layout ops array");
        let release_idx = ops
            .iter()
            .position(|op| op["data"].get("BankRelease").is_some())
            .expect("release op exists");
        ops.remove(release_idx);
        ops.retain(|op| op["data"].get("Yield").is_none());
        let invalid: Section = serde_json::from_value(json).expect("mutated section deserializes");

        assert!(matches!(
            validate_uses_banklease_ops(&invalid),
            Err(BringupValidationError::UnbalancedBankLeases { .. })
        ));
        assert!(matches!(
            validate_no_yield_while_lease_active(&invalid),
            Err(BringupValidationError::UnreleasedBankLeases { .. })
        ));
    }

    #[test]
    fn f_b1_banklease_validator_rejects_unknown_release() {
        let section = lower_request_to_asmir(&valid_distinct(64)).expect("section");
        let mut json = serde_json::to_value(&section).expect("section serializes");
        let ops = json["pre_layout_ops"]
            .as_array_mut()
            .expect("pre-layout ops array");
        let release = ops
            .iter_mut()
            .find_map(|op| op["data"].get_mut("BankRelease"))
            .expect("release op exists");
        release["lease_id"] = serde_json::json!(999_u32);
        let invalid: Section = serde_json::from_value(json).expect("mutated section deserializes");

        assert!(matches!(
            validate_no_yield_while_lease_active(&invalid),
            Err(BringupValidationError::UnknownBankRelease { lease_id: 999, .. })
        ));
    }

    #[test]
    fn f_b1_l2_generated_code_has_no_raw_mbc_writes() {
        let section = lower_request_to_asmir(&valid_distinct(64)).expect("section");
        validate_no_raw_mbc_writes(&section).expect("no raw mbc");
    }

    #[test]
    fn generated_code_has_no_raw_mbc_writes() {
        f_b1_l2_generated_code_has_no_raw_mbc_writes();
    }

    #[test]
    fn operand_copy_requires_active_lease() {
        let section = lower_request_to_asmir(&valid_distinct(64)).expect("section");
        let mut saw_copy_inside_lease = false;
        let mut active = false;
        for item in section.iter_items() {
            if let Some(effect) = item.machine_effect() {
                match effect.kind() {
                    MachineEffectKind::SystemCall => {}
                    MachineEffectKind::PureCompute if active => saw_copy_inside_lease = true,
                    _ => {}
                }
            }
            if let gbf_asm::section::SectionItemView::PreLayoutOp(op) = item {
                match op.data {
                    PreLayoutOp::BankLease(_) => active = true,
                    PreLayoutOp::BankRelease { .. } => active = false,
                    _ => {}
                }
            }
        }
        assert!(saw_copy_inside_lease);
    }

    #[test]
    fn f_b1_l3_tile_schedule_covers_output_once() {
        let dim = SquareDim::new(128).expect("valid");
        let schedule = tile_schedule(dim);
        assert_eq!(schedule.len(), 64);
        let unique: BTreeSet<_> = schedule.iter().map(|coord| (coord.mt, coord.nt)).collect();
        assert_eq!(unique.len(), 64);
    }

    #[test]
    fn f_b1_l3_arena_slots_do_not_overlap() {
        plan_arena().validate().expect("valid");
    }

    #[test]
    fn f_b1_streaming_rom_rejects_non_row_yield_quantum() {
        for yield_quantum in [YieldQuantum::KLaneRows4, YieldQuantum::KLaneFullTile] {
            let request = ComputeBringupRequest {
                yield_quantum,
                ..valid_distinct(32)
            };
            let error = build_l3_streaming_rom(&request).expect_err("non-row quantum rejected");
            assert!(error.to_string().contains("KLaneRow safe points"));
        }
    }

    #[test]
    fn f_b1_l1_lowering_is_deterministic() {
        let request = ComputeBringupRequest::l1_wram_smoke();
        assert_eq!(
            hash_json(&lower_request_to_asmir(&request).expect("section")),
            hash_json(&lower_request_to_asmir(&request).expect("section"))
        );
    }

    #[test]
    fn f_b1_report_aggregation_known_fixture() {
        let run = run_bringup_model(&valid_distinct(32)).expect("run");
        validate_structural_counts(&run.metrics, run.dim).expect("counts");
    }
}
