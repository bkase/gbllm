//! Typed canonical LR35902 instruction shapes.
//!
//! This module models the concrete, post-symbol-resolution instruction families
//! that the encoder will turn into bytes. Pseudo-ops, symbolic labels,
//! relocations, and branch relaxation are owned by later `gbf-asm` layers.
//!
//! Some values are intentionally canonical rather than a byte-for-byte mirror of
//! every legal CPU encoding. For example, high-memory A transfers use `LDH`
//! forms instead of the longer absolute `LD [imm16], A` / `LD A, [imm16]`
//! encodings. The raw encoder remains the only layer that emits opcode bytes;
//! this module defines the project AsmIR surface consumed by sizing, layout,
//! cycle accounting, and provenance.

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
