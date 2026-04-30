//! Public cycle-model adapter for typed LR35902 instructions.

pub use crate::isa::{CycleCost, TStateCost};

use crate::isa::Instr;

/// Static M-cycle cost for one canonical instruction shape.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost {
    instr.cycle_cost()
}

#[cfg(test)]
pub(crate) fn sample_instrs() -> Vec<Instr> {
    use crate::isa::{
        AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Reg8,
        Reg16Addr, Reg16Data, Reg16Stack, RstVector,
    };

    fn direct(addr: u16) -> DirectAddr {
        DirectAddr::new(addr).expect("valid direct address")
    }

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
            offset: HighDirectOffset::new(0x44),
        },
        Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(0x44),
        },
        Instr::LdAFromHighC,
        Instr::LdHighCFromA,
        Instr::Ld16Imm {
            dst: Reg16Data::HL,
            imm: 0xCAFE,
        },
        Instr::LdSpFromHl,
        Instr::LdDirectFromSp { addr: 0xC000 },
        Instr::LdHlFromSpPlus { off: -4 },
        Instr::AddA {
            src: AluSrc8::Reg(Reg8::B),
        },
        Instr::AddA {
            src: AluSrc8::HlIndirect,
        },
        Instr::AddA {
            src: AluSrc8::Imm(0x12),
        },
        Instr::Inc8 {
            dst: IncDec8Target::Reg(Reg8::B),
        },
        Instr::Inc8 {
            dst: IncDec8Target::HlIndirect,
        },
        Instr::Inc16 { dst: Reg16Data::BC },
        Instr::AddSp { off: -2 },
        Instr::Rlca,
        Instr::Rlc {
            target: CbTarget::Reg(Reg8::B),
        },
        Instr::Rlc {
            target: CbTarget::HlIndirect,
        },
        Instr::Bit {
            bit: BitIndex::B7,
            target: CbTarget::HlIndirect,
        },
        Instr::JpAbs {
            cond: None,
            addr: 0x0150,
        },
        Instr::JpAbs {
            cond: Some(Cond::NZ),
            addr: 0x4000,
        },
        Instr::JpHl,
        Instr::JrRel {
            cond: None,
            off: -2,
        },
        Instr::Call {
            cond: None,
            addr: 0x4000,
        },
        Instr::Call {
            cond: Some(Cond::C),
            addr: 0x4000,
        },
        Instr::Ret { cond: None },
        Instr::Ret {
            cond: Some(Cond::NC),
        },
        Instr::Reti,
        Instr::Rst {
            vector: RstVector::V38,
        },
        Instr::Push {
            src: Reg16Stack::AF,
        },
        Instr::Pop {
            dst: Reg16Stack::HL,
        },
    ]
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::*;
    use crate::isa::{
        AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Reg8,
        Reg16Addr, Reg16Data, Reg16Stack, RstVector,
    };

    fn direct(addr: u16) -> DirectAddr {
        DirectAddr::new(addr).expect("valid direct address")
    }

    fn fixed(cycles: u8) -> CycleCost {
        CycleCost::Fixed(NonZeroU8::new(cycles).expect("nonzero cycles"))
    }

    fn branch(taken: u8, not_taken: u8) -> CycleCost {
        CycleCost::Branch {
            taken: NonZeroU8::new(taken).expect("nonzero taken cycles"),
            not_taken: NonZeroU8::new(not_taken).expect("nonzero not-taken cycles"),
        }
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
    fn halt_one_mcycle() {
        assert_eq!(cycle_cost(&Instr::Halt), fixed(1));
    }
}
