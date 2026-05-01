//! Typed canonical LR35902 instruction shapes.
//!
//! This module models the concrete, post-symbol-resolution instruction families
//! that the encoder will turn into bytes. Structured ops, symbolic labels,
//! relocations, and branch relaxation are owned by later `gbf-asm` layers.
//!
//! Some values are intentionally canonical rather than a byte-for-byte mirror of
//! every legal CPU encoding. For example, high-memory A transfers use `LDH`
//! forms instead of the longer absolute `LD [imm16], A` / `LD A, [imm16]`
//! encodings. The raw encoder remains the only layer that emits opcode bytes;
//! this module defines the project AsmIR surface consumed by sizing, layout,
//! cycle accounting, and provenance.

use std::num::{NonZeroU8, NonZeroU16};

use serde::{Deserialize, Serialize};

/// Eight-bit CPU registers visible to generated code.
///
/// The flag register `F` is intentionally absent. It is only reachable through
/// the `AF` stack pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg8 {
    A,
    B,
    C,
    D,
    E,
    H,
    L,
}

/// Sixteen-bit register pairs and special registers.
///
/// Not every instruction accepts every variant. Use the narrower operand types
/// below (`Reg16Data`, `Reg16Stack`) at instruction boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg16 {
    BC,
    DE,
    HL,
    SP,
    AF,
}

/// Conditional branch predicates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Cond {
    NZ,
    Z,
    NC,
    C,
}

/// A typed three-bit index used by `BIT`, `RES`, and `SET`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct BitIndex(u8);

impl BitIndex {
    pub const B0: Self = Self(0);
    pub const B1: Self = Self(1);
    pub const B2: Self = Self(2);
    pub const B3: Self = Self(3);
    pub const B4: Self = Self(4);
    pub const B5: Self = Self(5);
    pub const B6: Self = Self(6);
    pub const B7: Self = Self(7);

    pub const fn new(value: u8) -> Option<Self> {
        if value < 8 { Some(Self(value)) } else { None }
    }

    pub const fn get(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for BitIndex {
    type Error = &'static str;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("bit index must be in 0..=7")
    }
}

impl From<BitIndex> for u8 {
    fn from(value: BitIndex) -> Self {
        value.get()
    }
}

/// Legal RST target vectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RstVector {
    V00,
    V08,
    V10,
    V18,
    V20,
    V28,
    V30,
    V38,
}

impl RstVector {
    pub const fn addr(self) -> u8 {
        match self {
            Self::V00 => 0x00,
            Self::V08 => 0x08,
            Self::V10 => 0x10,
            Self::V18 => 0x18,
            Self::V20 => 0x20,
            Self::V28 => 0x28,
            Self::V30 => 0x30,
            Self::V38 => 0x38,
        }
    }

    pub const fn new(addr: u8) -> Option<Self> {
        match addr {
            0x00 => Some(Self::V00),
            0x08 => Some(Self::V08),
            0x10 => Some(Self::V10),
            0x18 => Some(Self::V18),
            0x20 => Some(Self::V20),
            0x28 => Some(Self::V28),
            0x30 => Some(Self::V30),
            0x38 => Some(Self::V38),
            _ => None,
        }
    }
}

/// Direct 16-bit address used by canonical 8-bit A-memory transfers.
///
/// The `$FF00..=$FFFF` high-memory page has shorter `LDH` encodings. Keeping
/// that region out of `DirectAddr` makes the canonical lowering choice
/// structural instead of relying on an encoder warning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct DirectAddr(u16);

impl DirectAddr {
    pub const HIGH_MEMORY_START: u16 = 0xFF00;

    pub const fn new(addr: u16) -> Option<Self> {
        if addr < Self::HIGH_MEMORY_START {
            Some(Self(addr))
        } else {
            None
        }
    }

    pub const fn get(self) -> u16 {
        self.0
    }
}

impl TryFrom<u16> for DirectAddr {
    type Error = &'static str;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value).ok_or("direct A-transfer address must be below $FF00")
    }
}

impl From<DirectAddr> for u16 {
    fn from(value: DirectAddr) -> Self {
        value.get()
    }
}

/// `$FF00 + offset` high-memory address operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HighDirectOffset(u8);

impl HighDirectOffset {
    pub const fn new(offset: u8) -> Self {
        Self(offset)
    }

    pub const fn get(self) -> u8 {
        self.0
    }

    pub const fn absolute_addr(self) -> u16 {
        0xFF00 + self.0 as u16
    }
}

/// LR35902 8-bit operand classes used by instruction-family APIs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operand8Mode {
    Reg,
    Imm,
    HlIndirect,
    Direct,
    HighDirect,
    HighRegC,
}

/// LR35902 16-bit operand classes used by instruction-family APIs and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operand16Mode {
    Reg,
    Imm,
    Sp,
    SpPlus,
}

/// General 8-bit operand descriptor.
///
/// Instruction variants use narrower operand enums where direction matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operand8 {
    Reg(Reg8),
    Imm(u8),
    HlIndirect,
    Direct(DirectAddr),
    HighDirect(HighDirectOffset),
    HighRegC,
}

impl Operand8 {
    pub const ALL_MODES: [Operand8Mode; 6] = [
        Operand8Mode::Reg,
        Operand8Mode::Imm,
        Operand8Mode::HlIndirect,
        Operand8Mode::Direct,
        Operand8Mode::HighDirect,
        Operand8Mode::HighRegC,
    ];

    pub const fn mode(self) -> Operand8Mode {
        match self {
            Self::Reg(_) => Operand8Mode::Reg,
            Self::Imm(_) => Operand8Mode::Imm,
            Self::HlIndirect => Operand8Mode::HlIndirect,
            Self::Direct(_) => Operand8Mode::Direct,
            Self::HighDirect(_) => Operand8Mode::HighDirect,
            Self::HighRegC => Operand8Mode::HighRegC,
        }
    }
}

/// General 16-bit operand descriptor.
///
/// Instruction variants use narrower operand enums where register class
/// matters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Operand16 {
    RegPair(Reg16Pair),
    Imm(u16),
    Sp,
    SpPlus(i8),
}

impl Operand16 {
    pub const ALL_MODES: [Operand16Mode; 4] = [
        Operand16Mode::Reg,
        Operand16Mode::Imm,
        Operand16Mode::Sp,
        Operand16Mode::SpPlus,
    ];

    pub const fn mode(self) -> Operand16Mode {
        match self {
            Self::RegPair(_) => Operand16Mode::Reg,
            Self::Imm(_) => Operand16Mode::Imm,
            Self::Sp => Operand16Mode::Sp,
            Self::SpPlus(_) => Operand16Mode::SpPlus,
        }
    }
}

/// Sixteen-bit register pairs excluding the standalone stack pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg16Pair {
    BC,
    DE,
    HL,
    AF,
}

impl TryFrom<Reg16> for Reg16Pair {
    type Error = Reg16;

    fn try_from(value: Reg16) -> Result<Self, Self::Error> {
        match value {
            Reg16::BC => Ok(Self::BC),
            Reg16::DE => Ok(Self::DE),
            Reg16::HL => Ok(Self::HL),
            Reg16::AF => Ok(Self::AF),
            Reg16::SP => Err(value),
        }
    }
}

/// Register class accepted by Pan Docs' `r16` placeholder:
/// `LD rr, imm16`, `INC rr`, `DEC rr`, and `ADD HL, rr`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg16Data {
    BC,
    DE,
    HL,
    SP,
}

impl TryFrom<Reg16> for Reg16Data {
    type Error = Reg16;

    fn try_from(value: Reg16) -> Result<Self, Self::Error> {
        match value {
            Reg16::BC => Ok(Self::BC),
            Reg16::DE => Ok(Self::DE),
            Reg16::HL => Ok(Self::HL),
            Reg16::SP => Ok(Self::SP),
            Reg16::AF => Err(value),
        }
    }
}

/// Register pairs accepted by `PUSH` and `POP`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg16Stack {
    BC,
    DE,
    HL,
    AF,
}

impl TryFrom<Reg16> for Reg16Stack {
    type Error = Reg16;

    fn try_from(value: Reg16) -> Result<Self, Self::Error> {
        match value {
            Reg16::BC => Ok(Self::BC),
            Reg16::DE => Ok(Self::DE),
            Reg16::HL => Ok(Self::HL),
            Reg16::AF => Ok(Self::AF),
            Reg16::SP => Err(value),
        }
    }
}

/// Register-indirect A-transfer operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reg16Addr {
    BC,
    DE,
    Hli,
    Hld,
}

/// Source operand accepted by 8-bit ALU operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AluSrc8 {
    Reg(Reg8),
    HlIndirect,
    Imm(u8),
}

impl AluSrc8 {
    pub const fn byte_len(self) -> u8 {
        match self {
            Self::Reg(_) | Self::HlIndirect => 1,
            Self::Imm(_) => 2,
        }
    }
}

/// Target operand accepted by 8-bit increment/decrement operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IncDec8Target {
    Reg(Reg8),
    HlIndirect,
}

/// Target operand accepted by CB-prefixed rotate/shift/bit operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CbTarget {
    Reg(Reg8),
    HlIndirect,
}

/// Static M-cycle cost for one canonical instruction shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CycleCost {
    Fixed(NonZeroU8),
    Branch {
        taken: NonZeroU8,
        not_taken: NonZeroU8,
    },
}

impl CycleCost {
    #[must_use]
    pub const fn worst_case(self) -> u8 {
        match self {
            Self::Fixed(cycles) => cycles.get(),
            Self::Branch { taken, not_taken } => {
                if taken.get() >= not_taken.get() {
                    taken.get()
                } else {
                    not_taken.get()
                }
            }
        }
    }

    #[must_use]
    pub const fn best_case(self) -> u8 {
        match self {
            Self::Fixed(cycles) => cycles.get(),
            Self::Branch { taken, not_taken } => {
                if taken.get() <= not_taken.get() {
                    taken.get()
                } else {
                    not_taken.get()
                }
            }
        }
    }

    #[must_use]
    pub const fn t_states(self) -> TStateCost {
        match self {
            Self::Fixed(cycles) => TStateCost::Fixed(nz16(cycles.get() as u16 * 4)),
            Self::Branch { taken, not_taken } => TStateCost::Branch {
                taken: nz16(taken.get() as u16 * 4),
                not_taken: nz16(not_taken.get() as u16 * 4),
            },
        }
    }
}

/// T-state projection of a cycle cost. LR35902 timings are exactly `M * 4`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TStateCost {
    Fixed(NonZeroU16),
    Branch {
        taken: NonZeroU16,
        not_taken: NonZeroU16,
    },
}

/// Legal LR35902 instruction families.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Instr {
    Nop,
    Stop,
    Halt,
    Di,
    Ei,
    Ccf,
    Scf,
    Cpl,
    Daa,
    Ld8Reg { dst: Reg8, src: Reg8 },
    Ld8RegFromImm { dst: Reg8, imm: u8 },
    Ld8RegFromHl { dst: Reg8 },
    Ld8HlFromReg { src: Reg8 },
    Ld8HlFromImm { imm: u8 },
    LdAFromReg16Addr { src: Reg16Addr },
    LdReg16AddrFromA { dst: Reg16Addr },
    LdAFromDirect { addr: DirectAddr },
    LdDirectFromA { addr: DirectAddr },
    LdAFromHighDirect { offset: HighDirectOffset },
    LdHighDirectFromA { offset: HighDirectOffset },
    LdAFromHighC,
    LdHighCFromA,
    Ld16Imm { dst: Reg16Data, imm: u16 },
    LdSpFromHl,
    LdDirectFromSp { addr: u16 },
    LdHlFromSpPlus { off: i8 },
    AddA { src: AluSrc8 },
    AdcA { src: AluSrc8 },
    SubA { src: AluSrc8 },
    SbcA { src: AluSrc8 },
    AndA { src: AluSrc8 },
    OrA { src: AluSrc8 },
    XorA { src: AluSrc8 },
    CpA { src: AluSrc8 },
    Inc8 { dst: IncDec8Target },
    Dec8 { dst: IncDec8Target },
    Inc16 { dst: Reg16Data },
    Dec16 { dst: Reg16Data },
    AddHl { src: Reg16Data },
    AddSp { off: i8 },
    Rlca,
    Rrca,
    Rla,
    Rra,
    Rlc { target: CbTarget },
    Rrc { target: CbTarget },
    Rl { target: CbTarget },
    Rr { target: CbTarget },
    Sla { target: CbTarget },
    Sra { target: CbTarget },
    Srl { target: CbTarget },
    Swap { target: CbTarget },
    Bit { bit: BitIndex, target: CbTarget },
    Res { bit: BitIndex, target: CbTarget },
    Set { bit: BitIndex, target: CbTarget },
    JpAbs { cond: Option<Cond>, addr: u16 },
    JpHl,
    JrRel { cond: Option<Cond>, off: i8 },
    Call { cond: Option<Cond>, addr: u16 },
    Ret { cond: Option<Cond> },
    Reti,
    Rst { vector: RstVector },
    Push { src: Reg16Stack },
    Pop { dst: Reg16Stack },
}

impl Instr {
    /// Static M-cycle cost for this canonical instruction shape.
    #[must_use]
    pub const fn cycle_cost(self) -> CycleCost {
        match self {
            Self::Nop
            | Self::Stop
            | Self::Halt
            | Self::Di
            | Self::Ei
            | Self::Ccf
            | Self::Scf
            | Self::Cpl
            | Self::Daa
            | Self::Ld8Reg { .. }
            | Self::JpHl
            | Self::Rlca
            | Self::Rrca
            | Self::Rla
            | Self::Rra => fixed(1),
            Self::Ld8RegFromImm { .. }
            | Self::Ld8RegFromHl { .. }
            | Self::Ld8HlFromReg { .. }
            | Self::LdAFromHighC
            | Self::LdHighCFromA
            | Self::LdAFromReg16Addr { .. }
            | Self::LdReg16AddrFromA { .. }
            | Self::LdSpFromHl
            | Self::Inc16 { .. }
            | Self::Dec16 { .. }
            | Self::AddHl { .. } => fixed(2),
            Self::Ld8HlFromImm { .. }
            | Self::LdAFromHighDirect { .. }
            | Self::LdHighDirectFromA { .. }
            | Self::Ld16Imm { .. }
            | Self::LdHlFromSpPlus { .. } => fixed(3),
            Self::LdAFromDirect { .. }
            | Self::LdDirectFromA { .. }
            | Self::AddSp { .. }
            | Self::Ret { cond: None }
            | Self::Reti
            | Self::Rst { .. }
            | Self::Push { .. } => fixed(4),
            Self::LdDirectFromSp { .. } => fixed(5),
            Self::Call { cond: None, .. } => fixed(6),
            Self::AddA { src }
            | Self::AdcA { src }
            | Self::SubA { src }
            | Self::SbcA { src }
            | Self::AndA { src }
            | Self::OrA { src }
            | Self::XorA { src }
            | Self::CpA { src } => alu_src_cycle_cost(src),
            Self::Inc8 { dst } | Self::Dec8 { dst } => inc_dec_cycle_cost(dst),
            Self::Rlc { target }
            | Self::Rrc { target }
            | Self::Rl { target }
            | Self::Rr { target }
            | Self::Sla { target }
            | Self::Sra { target }
            | Self::Srl { target }
            | Self::Swap { target }
            | Self::Res { target, .. }
            | Self::Set { target, .. } => cb_rmw_cycle_cost(target),
            Self::Bit { target, .. } => cb_bit_cycle_cost(target),
            Self::JpAbs { cond: None, .. } => fixed(4),
            Self::JpAbs { cond: Some(_), .. } => branch(4, 3),
            Self::JrRel { cond: None, .. } => fixed(3),
            Self::JrRel { cond: Some(_), .. } => branch(3, 2),
            Self::Call { cond: Some(_), .. } => branch(6, 3),
            Self::Ret { cond: Some(_) } => branch(5, 2),
            Self::Pop { .. } => fixed(3),
        }
    }

    /// Encoded instruction length in bytes.
    ///
    /// The encoder remains the source of opcode bytes; this method is the
    /// layout/cycle-model preflight contract that every instruction shape has a
    /// known static width before branch relaxation.
    pub const fn byte_len(self) -> u8 {
        match self {
            Self::Nop
            | Self::Halt
            | Self::Di
            | Self::Ei
            | Self::Ccf
            | Self::Scf
            | Self::Cpl
            | Self::Daa
            | Self::Ld8Reg { .. }
            | Self::Ld8RegFromHl { .. }
            | Self::Ld8HlFromReg { .. }
            | Self::LdAFromReg16Addr { .. }
            | Self::LdReg16AddrFromA { .. }
            | Self::LdAFromHighC
            | Self::LdHighCFromA
            | Self::LdSpFromHl
            | Self::Inc8 { .. }
            | Self::Dec8 { .. }
            | Self::Inc16 { .. }
            | Self::Dec16 { .. }
            | Self::AddHl { .. }
            | Self::Rlca
            | Self::Rrca
            | Self::Rla
            | Self::Rra
            | Self::JpHl
            | Self::Ret { .. }
            | Self::Reti
            | Self::Rst { .. }
            | Self::Push { .. }
            | Self::Pop { .. } => 1,
            Self::Stop
            | Self::Ld8RegFromImm { .. }
            | Self::Ld8HlFromImm { .. }
            | Self::LdAFromHighDirect { .. }
            | Self::LdHighDirectFromA { .. }
            | Self::LdHlFromSpPlus { .. }
            | Self::AddSp { .. }
            | Self::Rlc { .. }
            | Self::Rrc { .. }
            | Self::Rl { .. }
            | Self::Rr { .. }
            | Self::Sla { .. }
            | Self::Sra { .. }
            | Self::Srl { .. }
            | Self::Swap { .. }
            | Self::Bit { .. }
            | Self::Res { .. }
            | Self::Set { .. }
            | Self::JrRel { .. } => 2,
            Self::LdAFromDirect { .. }
            | Self::LdDirectFromA { .. }
            | Self::Ld16Imm { .. }
            | Self::LdDirectFromSp { .. }
            | Self::JpAbs { .. }
            | Self::Call { .. } => 3,
            Self::AddA { src }
            | Self::AdcA { src }
            | Self::SubA { src }
            | Self::SbcA { src }
            | Self::AndA { src }
            | Self::OrA { src }
            | Self::XorA { src }
            | Self::CpA { src } => src.byte_len(),
        }
    }
}

/// Bytes plus canonical assembly mnemonic for one [`Instr`].
///
/// Returned by [`Instr::describe`]. Encoder and listing both consume this so
/// per-variant byte/mnemonic logic lives in exactly one match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstrDescriptor {
    pub(crate) bytes: Vec<u8>,
    pub(crate) mnemonic: String,
}

impl Instr {
    /// Single source of truth for per-variant encoding and assembly mnemonic.
    ///
    /// `here` is the CPU address of the instruction, used to resolve `JrRel`
    /// targets to an absolute hex string. Pass `0` if you only need bytes.
    #[must_use]
    pub(crate) fn describe(self, here: u16) -> InstrDescriptor {
        match self {
            Self::Nop => one(0x00, "nop"),
            Self::Stop => bytes_pair(0x10, 0x00, "stop"),
            Self::Halt => one(0x76, "halt"),
            Self::Di => one(0xF3, "di"),
            Self::Ei => one(0xFB, "ei"),
            Self::Ccf => one(0x3F, "ccf"),
            Self::Scf => one(0x37, "scf"),
            Self::Cpl => one(0x2F, "cpl"),
            Self::Daa => one(0x27, "daa"),
            Self::Ld8Reg { dst, src } => InstrDescriptor {
                bytes: vec![0x40 | (reg8_code(dst) << 3) | reg8_code(src)],
                mnemonic: format!("ld   {}, {}", reg8_name(dst), reg8_name(src)),
            },
            Self::Ld8RegFromImm { dst, imm } => InstrDescriptor {
                bytes: vec![0x06 | (reg8_code(dst) << 3), imm],
                mnemonic: format!("ld   {}, {}", reg8_name(dst), hex8(imm)),
            },
            Self::Ld8RegFromHl { dst } => InstrDescriptor {
                bytes: vec![0x46 | (reg8_code(dst) << 3)],
                mnemonic: format!("ld   {}, (hl)", reg8_name(dst)),
            },
            Self::Ld8HlFromReg { src } => InstrDescriptor {
                bytes: vec![0x70 | reg8_code(src)],
                mnemonic: format!("ld   (hl), {}", reg8_name(src)),
            },
            Self::Ld8HlFromImm { imm } => InstrDescriptor {
                bytes: vec![0x36, imm],
                mnemonic: format!("ld   (hl), {}", hex8(imm)),
            },
            Self::LdAFromReg16Addr { src } => {
                let opcode = match src {
                    Reg16Addr::BC => 0x0A,
                    Reg16Addr::DE => 0x1A,
                    Reg16Addr::Hli => 0x2A,
                    Reg16Addr::Hld => 0x3A,
                };
                InstrDescriptor {
                    bytes: vec![opcode],
                    mnemonic: format!("ld   a, {}", reg16_addr_name(src)),
                }
            }
            Self::LdReg16AddrFromA { dst } => {
                let opcode = match dst {
                    Reg16Addr::BC => 0x02,
                    Reg16Addr::DE => 0x12,
                    Reg16Addr::Hli => 0x22,
                    Reg16Addr::Hld => 0x32,
                };
                InstrDescriptor {
                    bytes: vec![opcode],
                    mnemonic: format!("ld   {}, a", reg16_addr_name(dst)),
                }
            }
            Self::LdAFromDirect { addr } => InstrDescriptor {
                bytes: u16_op(0xFA, addr.get()),
                mnemonic: format!("ld   a, ({})", hex16(addr.get())),
            },
            Self::LdDirectFromA { addr } => InstrDescriptor {
                bytes: u16_op(0xEA, addr.get()),
                mnemonic: format!("ld   ({}), a", hex16(addr.get())),
            },
            Self::LdAFromHighDirect { offset } => InstrDescriptor {
                bytes: vec![0xF0, offset.get()],
                mnemonic: format!("ldh  a, ({})", hex8(offset.get())),
            },
            Self::LdHighDirectFromA { offset } => InstrDescriptor {
                bytes: vec![0xE0, offset.get()],
                mnemonic: format!("ldh  ({}), a", hex8(offset.get())),
            },
            Self::LdAFromHighC => one(0xF2, "ldh  a, (c)"),
            Self::LdHighCFromA => one(0xE2, "ldh  (c), a"),
            Self::Ld16Imm { dst, imm } => InstrDescriptor {
                bytes: u16_op(0x01 | (reg16_data_code(dst) << 4), imm),
                mnemonic: format!("ld   {}, {}", reg16_data_name(dst), hex16(imm)),
            },
            Self::LdSpFromHl => one(0xF9, "ld   sp, hl"),
            Self::LdDirectFromSp { addr } => InstrDescriptor {
                bytes: u16_op(0x08, addr),
                mnemonic: format!("ld   ({}), sp", hex16(addr)),
            },
            Self::LdHlFromSpPlus { off } => InstrDescriptor {
                bytes: vec![0xF8, off as u8],
                mnemonic: format!("ld   hl, sp{off:+}"),
            },
            Self::AddA { src } => alu_describe(0x80, 0xC6, "add", src),
            Self::AdcA { src } => alu_describe(0x88, 0xCE, "adc", src),
            Self::SubA { src } => alu_describe(0x90, 0xD6, "sub", src),
            Self::SbcA { src } => alu_describe(0x98, 0xDE, "sbc", src),
            Self::AndA { src } => alu_describe(0xA0, 0xE6, "and", src),
            Self::XorA { src } => alu_describe(0xA8, 0xEE, "xor", src),
            Self::OrA { src } => alu_describe(0xB0, 0xF6, "or ", src),
            Self::CpA { src } => alu_describe(0xB8, 0xFE, "cp ", src),
            Self::Inc8 { dst } => {
                let opcode = match dst {
                    IncDec8Target::Reg(reg) => 0x04 | (reg8_code(reg) << 3),
                    IncDec8Target::HlIndirect => 0x34,
                };
                InstrDescriptor {
                    bytes: vec![opcode],
                    mnemonic: format!("inc  {}", inc_dec_name(dst)),
                }
            }
            Self::Dec8 { dst } => {
                let opcode = match dst {
                    IncDec8Target::Reg(reg) => 0x05 | (reg8_code(reg) << 3),
                    IncDec8Target::HlIndirect => 0x35,
                };
                InstrDescriptor {
                    bytes: vec![opcode],
                    mnemonic: format!("dec  {}", inc_dec_name(dst)),
                }
            }
            Self::Inc16 { dst } => InstrDescriptor {
                bytes: vec![0x03 | (reg16_data_code(dst) << 4)],
                mnemonic: format!("inc  {}", reg16_data_name(dst)),
            },
            Self::Dec16 { dst } => InstrDescriptor {
                bytes: vec![0x0B | (reg16_data_code(dst) << 4)],
                mnemonic: format!("dec  {}", reg16_data_name(dst)),
            },
            Self::AddHl { src } => InstrDescriptor {
                bytes: vec![0x09 | (reg16_data_code(src) << 4)],
                mnemonic: format!("add  hl, {}", reg16_data_name(src)),
            },
            Self::AddSp { off } => InstrDescriptor {
                bytes: vec![0xE8, off as u8],
                mnemonic: format!("add  sp, {off:+}"),
            },
            Self::Rlca => one(0x07, "rlca"),
            Self::Rrca => one(0x0F, "rrca"),
            Self::Rla => one(0x17, "rla"),
            Self::Rra => one(0x1F, "rra"),
            Self::Rlc { target } => cb_describe(0x00, "rlc ", target),
            Self::Rrc { target } => cb_describe(0x08, "rrc ", target),
            Self::Rl { target } => cb_describe(0x10, "rl  ", target),
            Self::Rr { target } => cb_describe(0x18, "rr  ", target),
            Self::Sla { target } => cb_describe(0x20, "sla ", target),
            Self::Sra { target } => cb_describe(0x28, "sra ", target),
            Self::Swap { target } => cb_describe(0x30, "swap", target),
            Self::Srl { target } => cb_describe(0x38, "srl ", target),
            Self::Bit { bit, target } => cb_bit_describe(0x40, "bit", bit, target),
            Self::Res { bit, target } => cb_bit_describe(0x80, "res", bit, target),
            Self::Set { bit, target } => cb_bit_describe(0xC0, "set", bit, target),
            Self::JpAbs { cond, addr } => InstrDescriptor {
                bytes: u16_op(jp_opcode(cond), addr),
                mnemonic: branch_abs_text("jp", cond, addr),
            },
            Self::JpHl => one(0xE9, "jp   hl"),
            Self::JrRel { cond, off } => {
                let target = here.wrapping_add(2).wrapping_add_signed(i16::from(off));
                InstrDescriptor {
                    bytes: vec![jr_opcode(cond), off as u8],
                    mnemonic: branch_rel_text("jr", cond, off, target),
                }
            }
            Self::Call { cond, addr } => InstrDescriptor {
                bytes: u16_op(call_opcode(cond), addr),
                mnemonic: branch_abs_text("call", cond, addr),
            },
            Self::Ret { cond } => {
                let mnemonic = match cond {
                    None => "ret".to_owned(),
                    Some(cond) => format!("ret  {}", cond_name(cond)),
                };
                InstrDescriptor {
                    bytes: vec![ret_opcode(cond)],
                    mnemonic,
                }
            }
            Self::Reti => one(0xD9, "reti"),
            Self::Rst { vector } => InstrDescriptor {
                bytes: vec![0xC7 | vector.addr()],
                mnemonic: format!("rst  {}", hex8(vector.addr())),
            },
            Self::Push { src } => InstrDescriptor {
                bytes: vec![0xC5 | (reg16_stack_code(src) << 4)],
                mnemonic: format!("push {}", reg16_stack_name(src)),
            },
            Self::Pop { dst } => InstrDescriptor {
                bytes: vec![0xC1 | (reg16_stack_code(dst) << 4)],
                mnemonic: format!("pop  {}", reg16_stack_name(dst)),
            },
        }
    }
}

fn one(byte: u8, mnem: &str) -> InstrDescriptor {
    InstrDescriptor {
        bytes: vec![byte],
        mnemonic: mnem.to_owned(),
    }
}

fn bytes_pair(a: u8, b: u8, mnem: &str) -> InstrDescriptor {
    InstrDescriptor {
        bytes: vec![a, b],
        mnemonic: mnem.to_owned(),
    }
}

fn u16_op(opcode: u8, value: u16) -> Vec<u8> {
    let bytes = value.to_le_bytes();
    vec![opcode, bytes[0], bytes[1]]
}

fn alu_describe(base: u8, imm_opcode: u8, op: &str, src: AluSrc8) -> InstrDescriptor {
    let bytes = match src {
        AluSrc8::Reg(reg) => vec![base | reg8_code(reg)],
        AluSrc8::HlIndirect => vec![base | 0x06],
        AluSrc8::Imm(imm) => vec![imm_opcode, imm],
    };
    InstrDescriptor {
        bytes,
        mnemonic: format!("{op}  a, {}", alu_src_name(src)),
    }
}

fn cb_describe(base: u8, op: &str, target: CbTarget) -> InstrDescriptor {
    InstrDescriptor {
        bytes: vec![0xCB, base | cb_target_code(target)],
        mnemonic: format!("{op} {}", cb_target_name(target)),
    }
}

fn cb_bit_describe(base: u8, op: &str, bit: BitIndex, target: CbTarget) -> InstrDescriptor {
    InstrDescriptor {
        bytes: vec![0xCB, base | (bit.get() << 3) | cb_target_code(target)],
        mnemonic: format!("{op:<5}{}, {}", bit.get(), cb_target_name(target)),
    }
}

fn branch_abs_text(op: &str, cond: Option<Cond>, addr: u16) -> String {
    match cond {
        None => format!("{op:<4} {}", hex16(addr)),
        Some(cond) => format!("{op:<4} {}, {}", cond_name(cond), hex16(addr)),
    }
}

fn branch_rel_text(op: &str, cond: Option<Cond>, off: i8, target: u16) -> String {
    match cond {
        None => format!("{op:<4} {off:+} ({})", hex16(target)),
        Some(cond) => format!("{op:<4} {}, {off:+} ({})", cond_name(cond), hex16(target)),
    }
}

fn reg8_code(reg: Reg8) -> u8 {
    match reg {
        Reg8::B => 0,
        Reg8::C => 1,
        Reg8::D => 2,
        Reg8::E => 3,
        Reg8::H => 4,
        Reg8::L => 5,
        Reg8::A => 7,
    }
}

fn reg8_name(reg: Reg8) -> &'static str {
    match reg {
        Reg8::A => "a",
        Reg8::B => "b",
        Reg8::C => "c",
        Reg8::D => "d",
        Reg8::E => "e",
        Reg8::H => "h",
        Reg8::L => "l",
    }
}

fn reg16_data_code(reg: Reg16Data) -> u8 {
    match reg {
        Reg16Data::BC => 0,
        Reg16Data::DE => 1,
        Reg16Data::HL => 2,
        Reg16Data::SP => 3,
    }
}

fn reg16_data_name(reg: Reg16Data) -> &'static str {
    match reg {
        Reg16Data::BC => "bc",
        Reg16Data::DE => "de",
        Reg16Data::HL => "hl",
        Reg16Data::SP => "sp",
    }
}

fn reg16_stack_code(reg: Reg16Stack) -> u8 {
    match reg {
        Reg16Stack::BC => 0,
        Reg16Stack::DE => 1,
        Reg16Stack::HL => 2,
        Reg16Stack::AF => 3,
    }
}

fn reg16_stack_name(reg: Reg16Stack) -> &'static str {
    match reg {
        Reg16Stack::BC => "bc",
        Reg16Stack::DE => "de",
        Reg16Stack::HL => "hl",
        Reg16Stack::AF => "af",
    }
}

fn reg16_addr_name(reg: Reg16Addr) -> &'static str {
    match reg {
        Reg16Addr::BC => "(bc)",
        Reg16Addr::DE => "(de)",
        Reg16Addr::Hli => "(hl+)",
        Reg16Addr::Hld => "(hl-)",
    }
}

fn alu_src_name(src: AluSrc8) -> String {
    match src {
        AluSrc8::Reg(reg) => reg8_name(reg).to_owned(),
        AluSrc8::HlIndirect => "(hl)".to_owned(),
        AluSrc8::Imm(imm) => hex8(imm),
    }
}

fn inc_dec_name(target: IncDec8Target) -> &'static str {
    match target {
        IncDec8Target::Reg(reg) => reg8_name(reg),
        IncDec8Target::HlIndirect => "(hl)",
    }
}

fn cb_target_code(target: CbTarget) -> u8 {
    match target {
        CbTarget::Reg(reg) => reg8_code(reg),
        CbTarget::HlIndirect => 6,
    }
}

fn cb_target_name(target: CbTarget) -> &'static str {
    match target {
        CbTarget::Reg(reg) => reg8_name(reg),
        CbTarget::HlIndirect => "(hl)",
    }
}

fn cond_name(cond: Cond) -> &'static str {
    match cond {
        Cond::NZ => "nz",
        Cond::Z => "z",
        Cond::NC => "nc",
        Cond::C => "c",
    }
}

fn jp_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xC3,
        Some(Cond::NZ) => 0xC2,
        Some(Cond::Z) => 0xCA,
        Some(Cond::NC) => 0xD2,
        Some(Cond::C) => 0xDA,
    }
}

fn jr_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0x18,
        Some(Cond::NZ) => 0x20,
        Some(Cond::Z) => 0x28,
        Some(Cond::NC) => 0x30,
        Some(Cond::C) => 0x38,
    }
}

fn call_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xCD,
        Some(Cond::NZ) => 0xC4,
        Some(Cond::Z) => 0xCC,
        Some(Cond::NC) => 0xD4,
        Some(Cond::C) => 0xDC,
    }
}

fn ret_opcode(cond: Option<Cond>) -> u8 {
    match cond {
        None => 0xC9,
        Some(Cond::NZ) => 0xC0,
        Some(Cond::Z) => 0xC8,
        Some(Cond::NC) => 0xD0,
        Some(Cond::C) => 0xD8,
    }
}

pub(crate) fn hex8(value: u8) -> String {
    format!("${value:02X}")
}

pub(crate) fn hex16(value: u16) -> String {
    format!("${value:04X}")
}

const fn alu_src_cycle_cost(src: AluSrc8) -> CycleCost {
    match src {
        AluSrc8::Reg(_) => fixed(1),
        AluSrc8::HlIndirect | AluSrc8::Imm(_) => fixed(2),
    }
}

const fn inc_dec_cycle_cost(dst: IncDec8Target) -> CycleCost {
    match dst {
        IncDec8Target::Reg(_) => fixed(1),
        IncDec8Target::HlIndirect => fixed(3),
    }
}

const fn cb_rmw_cycle_cost(target: CbTarget) -> CycleCost {
    match target {
        CbTarget::Reg(_) => fixed(2),
        CbTarget::HlIndirect => fixed(4),
    }
}

const fn cb_bit_cycle_cost(target: CbTarget) -> CycleCost {
    match target {
        CbTarget::Reg(_) => fixed(2),
        CbTarget::HlIndirect => fixed(3),
    }
}

const fn fixed(cycles: u8) -> CycleCost {
    CycleCost::Fixed(nz8(cycles))
}

const fn branch(taken: u8, not_taken: u8) -> CycleCost {
    CycleCost::Branch {
        taken: nz8(taken),
        not_taken: nz8(not_taken),
    }
}

const fn nz8(value: u8) -> NonZeroU8 {
    match NonZeroU8::new(value) {
        Some(value) => value,
        None => panic!("cycle cost must be nonzero"),
    }
}

const fn nz16(value: u16) -> NonZeroU16 {
    match NonZeroU16::new(value) {
        Some(value) => value,
        None => panic!("T-state cost must be nonzero"),
    }
}

#[cfg(test)]
#[test]
fn operand_classification() {
    assert_eq!(
        Operand8::ALL_MODES,
        [
            Operand8Mode::Reg,
            Operand8Mode::Imm,
            Operand8Mode::HlIndirect,
            Operand8Mode::Direct,
            Operand8Mode::HighDirect,
            Operand8Mode::HighRegC,
        ]
    );
    assert_eq!(
        Operand16::ALL_MODES,
        [
            Operand16Mode::Reg,
            Operand16Mode::Imm,
            Operand16Mode::Sp,
            Operand16Mode::SpPlus,
        ]
    );

    assert_eq!(Operand8::Reg(Reg8::A).mode(), Operand8Mode::Reg);
    assert_eq!(Operand8::Imm(7).mode(), Operand8Mode::Imm);
    assert_eq!(Operand8::HlIndirect.mode(), Operand8Mode::HlIndirect);
    assert_eq!(
        Operand8::Direct(DirectAddr::new(0xC000).expect("WRAM address is direct")).mode(),
        Operand8Mode::Direct
    );
    assert_eq!(
        Operand8::HighDirect(HighDirectOffset::new(0x80)).mode(),
        Operand8Mode::HighDirect
    );
    assert_eq!(Operand8::HighRegC.mode(), Operand8Mode::HighRegC);

    assert_eq!(Operand16::RegPair(Reg16Pair::HL).mode(), Operand16Mode::Reg);
    assert_eq!(Operand16::Imm(0x1234).mode(), Operand16Mode::Imm);
    assert_eq!(Operand16::Sp.mode(), Operand16Mode::Sp);
    assert_eq!(Operand16::SpPlus(-4).mode(), Operand16Mode::SpPlus);

    assert_eq!(
        DirectAddr::new(0xFEFF)
            .expect("last non-high address")
            .get(),
        0xFEFF
    );
    assert!(DirectAddr::new(0xFF00).is_none());
    assert!(DirectAddr::new(0xFFFF).is_none());
    assert_eq!(HighDirectOffset::new(0x42).absolute_addr(), 0xFF42);
    assert!(BitIndex::new(7).is_some());
    assert!(BitIndex::new(8).is_none());
    assert_eq!(RstVector::new(0x28).expect("valid rst").addr(), 0x28);
    assert!(RstVector::new(0x04).is_none());

    assert_eq!(Reg16Pair::try_from(Reg16::SP), Err(Reg16::SP));
    assert_eq!(Reg16Data::try_from(Reg16::AF), Err(Reg16::AF));
    assert_eq!(Reg16Stack::try_from(Reg16::SP), Err(Reg16::SP));
    assert_eq!(Reg16Stack::try_from(Reg16::AF), Ok(Reg16Stack::AF));
}

#[cfg(test)]
#[test]
fn serde_rejects_invalid_validated_operands() {
    assert!(serde_json::from_str::<BitIndex>("7").is_ok());
    assert!(serde_json::from_str::<BitIndex>("8").is_err());
    assert!(serde_json::from_str::<DirectAddr>("65278").is_ok());
    assert!(serde_json::from_str::<DirectAddr>("65280").is_err());

    let invalid_bit = r#"{"Bit":{"bit":8,"target":{"Reg":"A"}}}"#;
    assert!(serde_json::from_str::<Instr>(invalid_bit).is_err());

    let invalid_direct = r#"{"LdAFromDirect":{"addr":65280}}"#;
    assert!(serde_json::from_str::<Instr>(invalid_direct).is_err());
}

#[cfg(test)]
#[test]
fn instr_size_in_bytes() {
    let direct = DirectAddr::new(0xC123).expect("canonical direct address");
    let high = HighDirectOffset::new(0x80);

    let cases = [
        (Instr::Nop, 1),
        (Instr::Stop, 2),
        (
            Instr::Ld8Reg {
                dst: Reg8::A,
                src: Reg8::B,
            },
            1,
        ),
        (
            Instr::Ld8RegFromImm {
                dst: Reg8::C,
                imm: 3,
            },
            2,
        ),
        (Instr::Ld8RegFromHl { dst: Reg8::D }, 1),
        (Instr::Ld8HlFromReg { src: Reg8::E }, 1),
        (Instr::Ld8HlFromImm { imm: 0xF0 }, 2),
        (
            Instr::LdAFromReg16Addr {
                src: Reg16Addr::Hli,
            },
            1,
        ),
        (
            Instr::LdReg16AddrFromA {
                dst: Reg16Addr::Hld,
            },
            1,
        ),
        (Instr::LdAFromDirect { addr: direct }, 3),
        (Instr::LdDirectFromA { addr: direct }, 3),
        (Instr::LdAFromHighDirect { offset: high }, 2),
        (Instr::LdHighDirectFromA { offset: high }, 2),
        (Instr::LdAFromHighC, 1),
        (Instr::LdHighCFromA, 1),
        (
            Instr::Ld16Imm {
                dst: Reg16Data::HL,
                imm: 0xCAFE,
            },
            3,
        ),
        (Instr::LdSpFromHl, 1),
        (Instr::LdDirectFromSp { addr: 0xD000 }, 3),
        (Instr::LdHlFromSpPlus { off: -3 }, 2),
        (
            Instr::AddA {
                src: AluSrc8::Reg(Reg8::A),
            },
            1,
        ),
        (
            Instr::AdcA {
                src: AluSrc8::HlIndirect,
            },
            1,
        ),
        (
            Instr::SubA {
                src: AluSrc8::Imm(9),
            },
            2,
        ),
        (
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::B),
            },
            1,
        ),
        (
            Instr::AndA {
                src: AluSrc8::Imm(0x0F),
            },
            2,
        ),
        (
            Instr::OrA {
                src: AluSrc8::HlIndirect,
            },
            1,
        ),
        (
            Instr::XorA {
                src: AluSrc8::Reg(Reg8::C),
            },
            1,
        ),
        (
            Instr::CpA {
                src: AluSrc8::Imm(0x7F),
            },
            2,
        ),
        (
            Instr::Inc8 {
                dst: IncDec8Target::Reg(Reg8::L),
            },
            1,
        ),
        (
            Instr::Dec8 {
                dst: IncDec8Target::HlIndirect,
            },
            1,
        ),
        (Instr::Inc16 { dst: Reg16Data::BC }, 1),
        (Instr::Dec16 { dst: Reg16Data::SP }, 1),
        (Instr::AddHl { src: Reg16Data::DE }, 1),
        (Instr::AddSp { off: 4 }, 2),
        (Instr::Rlca, 1),
        (Instr::Rrca, 1),
        (Instr::Rla, 1),
        (Instr::Rra, 1),
        (
            Instr::Rlc {
                target: CbTarget::Reg(Reg8::B),
            },
            2,
        ),
        (
            Instr::Rrc {
                target: CbTarget::HlIndirect,
            },
            2,
        ),
        (
            Instr::Rl {
                target: CbTarget::Reg(Reg8::C),
            },
            2,
        ),
        (
            Instr::Rr {
                target: CbTarget::Reg(Reg8::D),
            },
            2,
        ),
        (
            Instr::Sla {
                target: CbTarget::Reg(Reg8::E),
            },
            2,
        ),
        (
            Instr::Sra {
                target: CbTarget::Reg(Reg8::H),
            },
            2,
        ),
        (
            Instr::Srl {
                target: CbTarget::Reg(Reg8::L),
            },
            2,
        ),
        (
            Instr::Swap {
                target: CbTarget::HlIndirect,
            },
            2,
        ),
        (
            Instr::Bit {
                bit: BitIndex::B7,
                target: CbTarget::Reg(Reg8::A),
            },
            2,
        ),
        (
            Instr::Res {
                bit: BitIndex::B0,
                target: CbTarget::HlIndirect,
            },
            2,
        ),
        (
            Instr::Set {
                bit: BitIndex::B3,
                target: CbTarget::Reg(Reg8::A),
            },
            2,
        ),
        (
            Instr::JpAbs {
                cond: Some(Cond::NZ),
                addr: 0x4000,
            },
            3,
        ),
        (Instr::JpHl, 1),
        (
            Instr::JrRel {
                cond: Some(Cond::C),
                off: -4,
            },
            2,
        ),
        (
            Instr::Call {
                cond: None,
                addr: 0x1234,
            },
            3,
        ),
        (
            Instr::Ret {
                cond: Some(Cond::Z),
            },
            1,
        ),
        (Instr::Reti, 1),
        (
            Instr::Rst {
                vector: RstVector::V38,
            },
            1,
        ),
        (
            Instr::Push {
                src: Reg16Stack::AF,
            },
            1,
        ),
        (
            Instr::Pop {
                dst: Reg16Stack::HL,
            },
            1,
        ),
        (Instr::Halt, 1),
        (Instr::Di, 1),
        (Instr::Ei, 1),
        (Instr::Ccf, 1),
        (Instr::Scf, 1),
        (Instr::Cpl, 1),
        (Instr::Daa, 1),
    ];

    for (instr, expected) in cases {
        assert_eq!(instr.byte_len(), expected, "{instr:?}");
    }
}

#[cfg(test)]
#[test]
fn describe_matches_gbdev_opcode_json() {
    use crate::test_support::gbdev_instr_cases;

    for case in gbdev_instr_cases() {
        let descriptor = case.instr().describe(0);
        assert_eq!(
            descriptor.bytes,
            case.expected_bytes(),
            "describe.bytes mismatch for {}",
            case.label()
        );
        let actual_op = descriptor
            .mnemonic
            .split_whitespace()
            .next()
            .expect("mnemonic must be nonempty");
        let expected_op = case.gbdev_mnemonic().to_lowercase();
        assert_eq!(
            actual_op,
            expected_op.as_str(),
            "describe.mnemonic op mismatch for {} (full mnemonic = {:?})",
            case.label(),
            descriptor.mnemonic
        );
    }
}
