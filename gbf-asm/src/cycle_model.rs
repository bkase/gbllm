//! Public cycle-model adapter for typed LR35902 instructions.

pub use crate::isa::{CycleCost, TStateCost};

use crate::isa::Instr;

/// Static M-cycle cost for one canonical instruction shape.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost {
    instr.cycle_cost()
}

#[cfg(test)]
use std::num::NonZeroU8;

#[cfg(test)]
use crate::isa::{
    AluSrc8, BitIndex, CbTarget, Cond, DirectAddr, HighDirectOffset, IncDec8Target, Reg8,
    Reg16Addr, Reg16Data, Reg16Stack, RstVector,
};
#[cfg(test)]
use crate::test_support::gbdev_instr_cases;

#[cfg(test)]
fn fixed(cycles: u8) -> CycleCost {
    CycleCost::Fixed(NonZeroU8::new(cycles).expect("nonzero cycles"))
}

#[cfg(test)]
fn branch(taken: u8, not_taken: u8) -> CycleCost {
    CycleCost::Branch {
        taken: NonZeroU8::new(taken).expect("nonzero taken cycles"),
        not_taken: NonZeroU8::new(not_taken).expect("nonzero not-taken cycles"),
    }
}

#[cfg(test)]
#[test]
fn cycle_model_matches_gbdev_opcode_json() {
    for case in gbdev_instr_cases() {
        let instr = case.instr();
        assert_eq!(
            cycle_cost(&instr),
            case.expected_cycle_cost(),
            "{}",
            case.label()
        );
    }
}

#[cfg(test)]
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

#[cfg(test)]
#[test]
fn t_states_lossless() {
    for case in gbdev_instr_cases() {
        let instr = case.instr();
        match (cycle_cost(&instr), cycle_cost(&instr).t_states()) {
            (CycleCost::Fixed(m), TStateCost::Fixed(t)) => {
                assert_eq!(u16::from(m.get()) * 4, t.get(), "{}", case.label());
                assert_eq!(t.get() / 4, u16::from(m.get()), "{}", case.label());
            }
            (
                CycleCost::Branch { taken, not_taken },
                TStateCost::Branch {
                    taken: t_taken,
                    not_taken: t_not_taken,
                },
            ) => {
                assert_eq!(
                    u16::from(taken.get()) * 4,
                    t_taken.get(),
                    "{}",
                    case.label()
                );
                assert_eq!(
                    u16::from(not_taken.get()) * 4,
                    t_not_taken.get(),
                    "{}",
                    case.label()
                );
            }
            pair => panic!("mismatched cycle/t-state shape: {pair:?}"),
        }
    }
}

#[cfg(test)]
#[test]
fn halt_one_mcycle() {
    assert_eq!(cycle_cost(&Instr::Halt), fixed(1));
}

#[cfg(test)]
#[test]
fn known_instructions() {
    let direct = DirectAddr::new(0xC000).expect("WRAM is a canonical direct address");
    let cases = [
        (Instr::Nop, fixed(1)),
        (Instr::Stop, fixed(1)),
        (Instr::Halt, fixed(1)),
        (Instr::Di, fixed(1)),
        (Instr::Ei, fixed(1)),
        (
            Instr::Ld8Reg {
                dst: Reg8::A,
                src: Reg8::B,
            },
            fixed(1),
        ),
        (
            Instr::Ld8RegFromImm {
                dst: Reg8::C,
                imm: 0x12,
            },
            fixed(2),
        ),
        (Instr::Ld8RegFromHl { dst: Reg8::D }, fixed(2)),
        (Instr::Ld8HlFromReg { src: Reg8::E }, fixed(2)),
        (Instr::Ld8HlFromImm { imm: 0x34 }, fixed(3)),
        (Instr::LdAFromReg16Addr { src: Reg16Addr::BC }, fixed(2)),
        (
            Instr::LdReg16AddrFromA {
                dst: Reg16Addr::Hli,
            },
            fixed(2),
        ),
        (Instr::LdAFromDirect { addr: direct }, fixed(4)),
        (Instr::LdDirectFromA { addr: direct }, fixed(4)),
        (
            Instr::LdAFromHighDirect {
                offset: HighDirectOffset::new(0x80),
            },
            fixed(3),
        ),
        (
            Instr::LdHighDirectFromA {
                offset: HighDirectOffset::new(0x80),
            },
            fixed(3),
        ),
        (Instr::LdAFromHighC, fixed(2)),
        (Instr::LdHighCFromA, fixed(2)),
        (
            Instr::Ld16Imm {
                dst: Reg16Data::HL,
                imm: 0x1234,
            },
            fixed(3),
        ),
        (Instr::LdSpFromHl, fixed(2)),
        (Instr::LdDirectFromSp { addr: 0xC000 }, fixed(5)),
        (Instr::LdHlFromSpPlus { off: -4 }, fixed(3)),
        (
            Instr::AddA {
                src: AluSrc8::Reg(Reg8::L),
            },
            fixed(1),
        ),
        (
            Instr::AdcA {
                src: AluSrc8::HlIndirect,
            },
            fixed(2),
        ),
        (
            Instr::CpA {
                src: AluSrc8::Imm(0x56),
            },
            fixed(2),
        ),
        (
            Instr::Inc8 {
                dst: IncDec8Target::Reg(Reg8::B),
            },
            fixed(1),
        ),
        (
            Instr::Dec8 {
                dst: IncDec8Target::HlIndirect,
            },
            fixed(3),
        ),
        (Instr::Inc16 { dst: Reg16Data::SP }, fixed(2)),
        (Instr::AddHl { src: Reg16Data::DE }, fixed(2)),
        (Instr::AddSp { off: 7 }, fixed(4)),
        (Instr::Rlca, fixed(1)),
        (
            Instr::Rlc {
                target: CbTarget::Reg(Reg8::A),
            },
            fixed(2),
        ),
        (
            Instr::Swap {
                target: CbTarget::HlIndirect,
            },
            fixed(4),
        ),
        (
            Instr::Bit {
                bit: BitIndex::B7,
                target: CbTarget::HlIndirect,
            },
            fixed(3),
        ),
        (
            Instr::Set {
                bit: BitIndex::B0,
                target: CbTarget::Reg(Reg8::C),
            },
            fixed(2),
        ),
        (
            Instr::JpAbs {
                cond: None,
                addr: 0x1234,
            },
            fixed(4),
        ),
        (
            Instr::JpAbs {
                cond: Some(Cond::Z),
                addr: 0x1234,
            },
            branch(4, 3),
        ),
        (Instr::JpHl, fixed(1)),
        (Instr::JrRel { cond: None, off: 8 }, fixed(3)),
        (
            Instr::JrRel {
                cond: Some(Cond::NZ),
                off: -8,
            },
            branch(3, 2),
        ),
        (
            Instr::Call {
                cond: None,
                addr: 0x1234,
            },
            fixed(6),
        ),
        (
            Instr::Call {
                cond: Some(Cond::C),
                addr: 0x1234,
            },
            branch(6, 3),
        ),
        (Instr::Ret { cond: None }, fixed(4)),
        (
            Instr::Ret {
                cond: Some(Cond::NC),
            },
            branch(5, 2),
        ),
        (Instr::Reti, fixed(4)),
        (
            Instr::Rst {
                vector: RstVector::V38,
            },
            fixed(4),
        ),
        (
            Instr::Push {
                src: Reg16Stack::AF,
            },
            fixed(4),
        ),
        (
            Instr::Pop {
                dst: Reg16Stack::BC,
            },
            fixed(3),
        ),
    ];

    assert!(cases.len() >= 30, "spot check should stay broad");
    for (instr, expected) in cases {
        assert_eq!(cycle_cost(&instr), expected, "{instr:?}");
    }
}

#[cfg(test)]
#[test]
fn no_zero_cost() {
    let mut checked = 0;
    for case in gbdev_instr_cases() {
        checked += 1;
        let instr = case.instr();
        match cycle_cost(&instr) {
            CycleCost::Fixed(cycles) => {
                assert_ne!(cycles.get(), 0, "{}", case.label());
            }
            CycleCost::Branch { taken, not_taken } => {
                assert_ne!(taken.get(), 0, "{} taken", case.label());
                assert_ne!(not_taken.get(), 0, "{} not taken", case.label());
            }
        }
    }
    assert_eq!(checked, 500, "all legal gbdev opcodes are checked");
}

#[cfg(test)]
#[test]
fn branch_invariant() {
    for case in gbdev_instr_cases() {
        if let CycleCost::Branch { taken, not_taken } = cycle_cost(&case.instr()) {
            assert!(
                taken.get() > not_taken.get(),
                "{} should cost more when taken",
                case.label()
            );
        }
    }
}
