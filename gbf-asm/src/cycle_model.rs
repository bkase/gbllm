//! Public cycle-model adapter for typed LR35902 instructions.

pub use crate::isa::{CycleCost, TStateCost};

use crate::isa::Instr;

/// Static M-cycle cost for one canonical instruction shape.
#[must_use]
pub const fn cycle_cost(instr: &Instr) -> CycleCost {
    instr.cycle_cost()
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU8;

    use super::*;
    use crate::isa::Cond;
    use crate::test_support::gbdev_instr_cases;

    fn fixed(cycles: u8) -> CycleCost {
        CycleCost::Fixed(NonZeroU8::new(cycles).expect("nonzero cycles"))
    }

    fn branch(taken: u8, not_taken: u8) -> CycleCost {
        CycleCost::Branch {
            taken: NonZeroU8::new(taken).expect("nonzero taken cycles"),
            not_taken: NonZeroU8::new(not_taken).expect("nonzero not-taken cycles"),
        }
    }

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

    #[test]
    fn halt_one_mcycle() {
        assert_eq!(cycle_cost(&Instr::Halt), fixed(1));
    }
}
