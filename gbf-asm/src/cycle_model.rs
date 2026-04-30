//! Static LR35902 cycle costs for typed instructions.

use std::num::{NonZeroU8, NonZeroU16};

use serde::{Deserialize, Serialize};

use crate::isa::{AluSrc8, CbTarget, IncDec8Target, Instr};

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

/// Pure static M-cycle table for every typed instruction variant.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost {
    match *instr {
        Instr::Nop
        | Instr::Stop
        | Instr::Halt
        | Instr::Di
        | Instr::Ei
        | Instr::Ccf
        | Instr::Scf
        | Instr::Cpl
        | Instr::Daa
        | Instr::Ld8Reg { .. }
        | Instr::JpHl
        | Instr::Rlca
        | Instr::Rrca
        | Instr::Rla
        | Instr::Rra => fixed(1),
        Instr::Ld8RegFromImm { .. }
        | Instr::Ld8RegFromHl { .. }
        | Instr::Ld8HlFromReg { .. }
        | Instr::LdAFromHighC
        | Instr::LdHighCFromA
        | Instr::LdAFromReg16Addr { .. }
        | Instr::LdReg16AddrFromA { .. }
        | Instr::LdSpFromHl
        | Instr::Inc16 { .. }
        | Instr::Dec16 { .. }
        | Instr::AddHl { .. } => fixed(2),
        Instr::Ld8HlFromImm { .. }
        | Instr::LdAFromHighDirect { .. }
        | Instr::LdHighDirectFromA { .. }
        | Instr::Ld16Imm { .. }
        | Instr::LdHlFromSpPlus { .. } => fixed(3),
        Instr::LdAFromDirect { .. }
        | Instr::LdDirectFromA { .. }
        | Instr::AddSp { .. }
        | Instr::Ret { cond: None }
        | Instr::Reti
        | Instr::Rst { .. }
        | Instr::Push { .. } => fixed(4),
        Instr::LdDirectFromSp { .. } => fixed(5),
        Instr::Call { cond: None, .. } => fixed(6),
        Instr::AddA { src }
        | Instr::AdcA { src }
        | Instr::SubA { src }
        | Instr::SbcA { src }
        | Instr::AndA { src }
        | Instr::OrA { src }
        | Instr::XorA { src }
        | Instr::CpA { src } => alu_src_cost(src),
        Instr::Inc8 { dst } | Instr::Dec8 { dst } => inc_dec_cost(dst),
        Instr::Rlc { target }
        | Instr::Rrc { target }
        | Instr::Rl { target }
        | Instr::Rr { target }
        | Instr::Sla { target }
        | Instr::Sra { target }
        | Instr::Srl { target }
        | Instr::Swap { target }
        | Instr::Res { target, .. }
        | Instr::Set { target, .. } => cb_rmw_cost(target),
        Instr::Bit { target, .. } => cb_bit_cost(target),
        Instr::JpAbs { cond: None, .. } => fixed(4),
        Instr::JpAbs { cond: Some(_), .. } => branch(4, 3),
        Instr::JrRel { cond: None, .. } => fixed(3),
        Instr::JrRel { cond: Some(_), .. } => branch(3, 2),
        Instr::Call { cond: Some(_), .. } => branch(6, 3),
        Instr::Ret { cond: Some(_) } => branch(5, 2),
        Instr::Pop { .. } => fixed(3),
    }
}

const fn alu_src_cost(src: AluSrc8) -> CycleCost {
    match src {
        AluSrc8::Reg(_) => fixed(1),
        AluSrc8::HlIndirect | AluSrc8::Imm(_) => fixed(2),
    }
}

const fn inc_dec_cost(dst: IncDec8Target) -> CycleCost {
    match dst {
        IncDec8Target::Reg(_) => fixed(1),
        IncDec8Target::HlIndirect => fixed(3),
    }
}

const fn cb_rmw_cost(target: CbTarget) -> CycleCost {
    match target {
        CbTarget::Reg(_) => fixed(2),
        CbTarget::HlIndirect => fixed(4),
    }
}

const fn cb_bit_cost(target: CbTarget) -> CycleCost {
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
mod tests {
    use super::*;
    use crate::isa::{
        AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Instr,
        Reg8, Reg16Addr, Reg16Data, Reg16Stack, RstVector,
    };

    fn direct(addr: u16) -> DirectAddr {
        DirectAddr::new(addr).expect("valid direct address")
    }

    fn fixed_cycles(instr: Instr) -> u8 {
        match cycle_cost(&instr) {
            CycleCost::Fixed(cycles) => cycles.get(),
            CycleCost::Branch { .. } => panic!("{instr:?} has branch cost"),
        }
    }

    #[test]
    fn known_instructions() {
        let cases = [
            (Instr::Nop, 1),
            (Instr::Stop, 1),
            (Instr::Halt, 1),
            (Instr::Di, 1),
            (Instr::Ei, 1),
            (
                Instr::Ld8Reg {
                    dst: Reg8::A,
                    src: Reg8::B,
                },
                1,
            ),
            (
                Instr::Ld8RegFromImm {
                    dst: Reg8::A,
                    imm: 0x42,
                },
                2,
            ),
            (Instr::Ld8RegFromHl { dst: Reg8::A }, 2),
            (Instr::Ld8HlFromReg { src: Reg8::A }, 2),
            (Instr::Ld8HlFromImm { imm: 0x12 }, 3),
            (
                Instr::LdAFromReg16Addr {
                    src: Reg16Addr::Hli,
                },
                2,
            ),
            (
                Instr::LdReg16AddrFromA {
                    dst: Reg16Addr::Hld,
                },
                2,
            ),
            (
                Instr::LdAFromDirect {
                    addr: direct(0xC000),
                },
                4,
            ),
            (
                Instr::LdDirectFromA {
                    addr: direct(0xC000),
                },
                4,
            ),
            (
                Instr::LdAFromHighDirect {
                    offset: HighDirectOffset::new(0x80),
                },
                3,
            ),
            (Instr::LdAFromHighC, 2),
            (Instr::LdHighCFromA, 2),
            (
                Instr::Ld16Imm {
                    dst: Reg16Data::HL,
                    imm: 0xCAFE,
                },
                3,
            ),
            (Instr::LdDirectFromSp { addr: 0xC000 }, 5),
            (
                Instr::AddA {
                    src: AluSrc8::Reg(Reg8::B),
                },
                1,
            ),
            (
                Instr::AddA {
                    src: AluSrc8::HlIndirect,
                },
                2,
            ),
            (
                Instr::AddA {
                    src: AluSrc8::Imm(1),
                },
                2,
            ),
            (
                Instr::Inc8 {
                    dst: IncDec8Target::Reg(Reg8::B),
                },
                1,
            ),
            (
                Instr::Inc8 {
                    dst: IncDec8Target::HlIndirect,
                },
                3,
            ),
            (Instr::Inc16 { dst: Reg16Data::BC }, 2),
            (Instr::AddSp { off: -4 }, 4),
            (Instr::Rlca, 1),
            (
                Instr::Rlc {
                    target: CbTarget::Reg(Reg8::B),
                },
                2,
            ),
            (
                Instr::Rlc {
                    target: CbTarget::HlIndirect,
                },
                4,
            ),
            (
                Instr::Bit {
                    bit: BitIndex::B7,
                    target: CbTarget::HlIndirect,
                },
                3,
            ),
            (
                Instr::JpAbs {
                    cond: None,
                    addr: 0x0150,
                },
                4,
            ),
            (Instr::JpHl, 1),
            (
                Instr::JrRel {
                    cond: None,
                    off: -2,
                },
                3,
            ),
            (
                Instr::Call {
                    cond: None,
                    addr: 0x4000,
                },
                6,
            ),
            (Instr::Ret { cond: None }, 4),
            (Instr::Reti, 4),
            (
                Instr::Rst {
                    vector: RstVector::V38,
                },
                4,
            ),
            (
                Instr::Push {
                    src: Reg16Stack::AF,
                },
                4,
            ),
            (
                Instr::Pop {
                    dst: Reg16Stack::HL,
                },
                3,
            ),
        ];

        for (instr, expected) in cases {
            assert_eq!(fixed_cycles(instr), expected, "{instr:?}");
        }
    }

    #[test]
    fn conditional_branch_timings_by_family() {
        assert_eq!(
            cycle_cost(&Instr::JrRel {
                cond: Some(Cond::NZ),
                off: 4,
            }),
            branch(3, 2)
        );
        assert_eq!(
            cycle_cost(&Instr::JpAbs {
                cond: Some(Cond::Z),
                addr: 0x1234,
            }),
            branch(4, 3)
        );
        assert_eq!(
            cycle_cost(&Instr::Call {
                cond: Some(Cond::C),
                addr: 0x1234,
            }),
            branch(6, 3)
        );
        assert_eq!(
            cycle_cost(&Instr::Ret {
                cond: Some(Cond::NC)
            }),
            branch(5, 2)
        );
    }

    #[test]
    fn t_states_lossless() {
        for instr in sample_instrs() {
            match (cycle_cost(&instr), cycle_cost(&instr).t_states()) {
                (CycleCost::Fixed(m), TStateCost::Fixed(t)) => {
                    assert_eq!(u16::from(m.get()) * 4, t.get());
                    assert_eq!(t.get() / 4, u16::from(m.get()));
                }
                (
                    CycleCost::Branch { taken, not_taken },
                    TStateCost::Branch {
                        taken: t_taken,
                        not_taken: t_not_taken,
                    },
                ) => {
                    assert_eq!(u16::from(taken.get()) * 4, t_taken.get());
                    assert_eq!(u16::from(not_taken.get()) * 4, t_not_taken.get());
                }
                pair => panic!("mismatched cycle/t-state shape: {pair:?}"),
            }
        }
    }

    #[test]
    fn no_zero_cost() {
        for instr in sample_instrs() {
            assert!(cycle_cost(&instr).best_case() > 0, "{instr:?}");
            assert!(cycle_cost(&instr).worst_case() > 0, "{instr:?}");
        }
    }

    #[test]
    fn halt_one_mcycle() {
        assert_eq!(cycle_cost(&Instr::Halt), fixed(1));
    }

    fn sample_instrs() -> Vec<Instr> {
        vec![
            Instr::Nop,
            Instr::Stop,
            Instr::Halt,
            Instr::Di,
            Instr::Ei,
            Instr::Ccf,
            Instr::Scf,
            Instr::Cpl,
            Instr::Daa,
            Instr::Ld8Reg {
                dst: Reg8::A,
                src: Reg8::B,
            },
            Instr::Ld8RegFromImm {
                dst: Reg8::A,
                imm: 0x12,
            },
            Instr::Ld8RegFromHl { dst: Reg8::B },
            Instr::Ld8HlFromReg { src: Reg8::C },
            Instr::Ld8HlFromImm { imm: 0x34 },
            Instr::LdAFromReg16Addr { src: Reg16Addr::BC },
            Instr::LdReg16AddrFromA { dst: Reg16Addr::DE },
            Instr::LdAFromDirect {
                addr: direct(0x1234),
            },
            Instr::LdDirectFromA {
                addr: direct(0xC000),
            },
            Instr::LdAFromHighDirect {
                offset: HighDirectOffset::new(0x40),
            },
            Instr::LdHighDirectFromA {
                offset: HighDirectOffset::new(0x80),
            },
            Instr::LdAFromHighC,
            Instr::LdHighCFromA,
            Instr::Ld16Imm {
                dst: Reg16Data::SP,
                imm: 0xFFFE,
            },
            Instr::LdSpFromHl,
            Instr::LdDirectFromSp { addr: 0xC000 },
            Instr::LdHlFromSpPlus { off: -1 },
            Instr::AddA {
                src: AluSrc8::Reg(Reg8::B),
            },
            Instr::AdcA {
                src: AluSrc8::HlIndirect,
            },
            Instr::SubA {
                src: AluSrc8::Imm(7),
            },
            Instr::SbcA {
                src: AluSrc8::Reg(Reg8::C),
            },
            Instr::AndA {
                src: AluSrc8::Imm(0xF0),
            },
            Instr::OrA {
                src: AluSrc8::Reg(Reg8::D),
            },
            Instr::XorA {
                src: AluSrc8::HlIndirect,
            },
            Instr::CpA {
                src: AluSrc8::Imm(0),
            },
            Instr::Inc8 {
                dst: IncDec8Target::Reg(Reg8::E),
            },
            Instr::Dec8 {
                dst: IncDec8Target::HlIndirect,
            },
            Instr::Inc16 { dst: Reg16Data::BC },
            Instr::Dec16 { dst: Reg16Data::DE },
            Instr::AddHl { src: Reg16Data::HL },
            Instr::AddSp { off: 1 },
            Instr::Rlca,
            Instr::Rrca,
            Instr::Rla,
            Instr::Rra,
            Instr::Rlc {
                target: CbTarget::Reg(Reg8::H),
            },
            Instr::Rrc {
                target: CbTarget::HlIndirect,
            },
            Instr::Rl {
                target: CbTarget::Reg(Reg8::L),
            },
            Instr::Rr {
                target: CbTarget::HlIndirect,
            },
            Instr::Sla {
                target: CbTarget::Reg(Reg8::A),
            },
            Instr::Sra {
                target: CbTarget::HlIndirect,
            },
            Instr::Srl {
                target: CbTarget::Reg(Reg8::B),
            },
            Instr::Swap {
                target: CbTarget::HlIndirect,
            },
            Instr::Bit {
                bit: BitIndex::B3,
                target: CbTarget::Reg(Reg8::D),
            },
            Instr::Res {
                bit: BitIndex::B2,
                target: CbTarget::HlIndirect,
            },
            Instr::Set {
                bit: BitIndex::B1,
                target: CbTarget::Reg(Reg8::E),
            },
            Instr::JpAbs {
                cond: None,
                addr: 0x1234,
            },
            Instr::JpAbs {
                cond: Some(Cond::NZ),
                addr: 0x1234,
            },
            Instr::JpHl,
            Instr::JrRel { cond: None, off: 1 },
            Instr::JrRel {
                cond: Some(Cond::C),
                off: -4,
            },
            Instr::Call {
                cond: None,
                addr: 0x1234,
            },
            Instr::Call {
                cond: Some(Cond::NC),
                addr: 0x1234,
            },
            Instr::Ret { cond: None },
            Instr::Ret {
                cond: Some(Cond::Z),
            },
            Instr::Reti,
            Instr::Rst {
                vector: RstVector::V20,
            },
            Instr::Push {
                src: Reg16Stack::BC,
            },
            Instr::Pop {
                dst: Reg16Stack::AF,
            },
        ]
    }
}
