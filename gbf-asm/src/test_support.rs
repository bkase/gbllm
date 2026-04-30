use std::collections::BTreeMap;
use std::num::NonZeroU8;

use serde::Deserialize;

use crate::isa::{
    AluSrc8, BitIndex, CbTarget, Cond, CycleCost, DirectAddr, HighDirectOffset, IncDec8Target,
    Instr, Reg8, Reg16Addr, Reg16Data, Reg16Stack, RstVector,
};

const GBDEV_OPCODE_JSON: &str = include_str!("../tests/fixtures/gbdev-opcodes.json");
const SAMPLE_N8: u8 = 0x42;
const SAMPLE_A8: u8 = 0x44;
const SAMPLE_I8: i8 = -2;
const SAMPLE_U16: u16 = 0x1234;

#[derive(Debug, Deserialize)]
struct GbdevOpcodeTable {
    unprefixed: BTreeMap<String, GbdevOpcode>,
    cbprefixed: BTreeMap<String, GbdevOpcode>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbdevOpcode {
    mnemonic: String,
    bytes: u8,
    cycles: Vec<u8>,
    #[serde(default)]
    operands: Vec<GbdevOperand>,
}

#[derive(Debug, Clone, Deserialize)]
struct GbdevOperand {
    name: String,
    #[serde(default)]
    immediate: bool,
    bytes: Option<u8>,
}

#[derive(Debug, Clone)]
pub(crate) struct GbdevInstrCase {
    opcode: u8,
    prefixed: bool,
    spec: GbdevOpcode,
    instr: Instr,
}

impl GbdevInstrCase {
    pub(crate) fn instr(&self) -> Instr {
        self.instr
    }

    pub(crate) fn is_prefixed(&self) -> bool {
        self.prefixed
    }

    pub(crate) fn expected_byte_len(&self) -> u8 {
        self.spec.bytes
    }

    pub(crate) fn expected_bytes(&self) -> Vec<u8> {
        let mut bytes = if self.prefixed {
            vec![0xCB, self.opcode]
        } else {
            vec![self.opcode]
        };
        if !self.prefixed {
            for operand in &self.spec.operands {
                match (operand.name.as_str(), operand.bytes) {
                    ("n8", Some(1)) if self.spec.mnemonic == "STOP" => bytes.push(0x00),
                    ("n8", Some(1)) => bytes.push(SAMPLE_N8),
                    ("a8", Some(1)) => bytes.push(SAMPLE_A8),
                    ("e8", Some(1)) => bytes.push(SAMPLE_I8 as u8),
                    ("n16" | "a16", Some(2)) => bytes.extend_from_slice(&SAMPLE_U16.to_le_bytes()),
                    (_, None) => {}
                    _ => panic!("unsupported gbdev immediate operand in {}", self.label()),
                }
            }
        }
        bytes
    }

    pub(crate) fn expected_cycle_cost(&self) -> CycleCost {
        let m_cycles: Vec<u8> = self
            .spec
            .cycles
            .iter()
            .map(|cycles| {
                assert_eq!(
                    cycles % 4,
                    0,
                    "gbdev cycle count must be divisible by 4 in {}",
                    self.label()
                );
                cycles / 4
            })
            .collect();
        match m_cycles.as_slice() {
            [cycles] => CycleCost::Fixed(nonzero(*cycles)),
            [taken, not_taken] => CycleCost::Branch {
                taken: nonzero(*taken),
                not_taken: nonzero(*not_taken),
            },
            _ => panic!("unsupported gbdev cycle shape in {}", self.label()),
        }
    }

    pub(crate) fn label(&self) -> String {
        let prefix = if self.prefixed { "CB " } else { "" };
        let operands = self
            .spec
            .operands
            .iter()
            .map(|operand| {
                if operand.immediate {
                    operand.name.clone()
                } else {
                    format!("({})", operand.name)
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        if operands.is_empty() {
            format!("{prefix}0x{:02X} {}", self.opcode, self.spec.mnemonic)
        } else {
            format!(
                "{prefix}0x{:02X} {} {}",
                self.opcode, self.spec.mnemonic, operands
            )
        }
    }
}

pub(crate) fn gbdev_instr_cases() -> Vec<GbdevInstrCase> {
    let table: GbdevOpcodeTable =
        serde_json::from_str(GBDEV_OPCODE_JSON).expect("gbdev opcode JSON fixture parses");
    let mut cases = Vec::new();
    let mut unprefixed_cases = 0;
    let mut cb_cases = 0;

    for (key, spec) in table.unprefixed {
        let opcode = parse_opcode_key(&key);
        match instr_from_unprefixed(opcode, &spec) {
            Some(instr) => {
                unprefixed_cases += 1;
                cases.push(GbdevInstrCase {
                    opcode,
                    prefixed: false,
                    spec,
                    instr,
                });
            }
            None => assert!(
                spec.mnemonic == "PREFIX" || spec.mnemonic.starts_with("ILLEGAL_"),
                "unsupported legal gbdev opcode {key}: {spec:?}"
            ),
        }
    }

    for (key, spec) in table.cbprefixed {
        let opcode = parse_opcode_key(&key);
        let instr = instr_from_cbprefixed(&spec);
        cb_cases += 1;
        cases.push(GbdevInstrCase {
            opcode,
            prefixed: true,
            spec,
            instr,
        });
    }

    assert_eq!(unprefixed_cases, 244, "gbdev legal unprefixed opcode count");
    assert_eq!(cb_cases, 256, "gbdev CB-prefixed opcode count");
    cases
}

fn instr_from_unprefixed(opcode: u8, spec: &GbdevOpcode) -> Option<Instr> {
    match spec.mnemonic.as_str() {
        "NOP" => Some(Instr::Nop),
        "STOP" => Some(Instr::Stop),
        "HALT" => Some(Instr::Halt),
        "DI" => Some(Instr::Di),
        "EI" => Some(Instr::Ei),
        "CCF" => Some(Instr::Ccf),
        "SCF" => Some(Instr::Scf),
        "CPL" => Some(Instr::Cpl),
        "DAA" => Some(Instr::Daa),
        "LD" | "LDH" => Some(ld_instr(opcode, spec)),
        "INC" => Some(inc_instr(spec)),
        "DEC" => Some(dec_instr(spec)),
        "ADD" => Some(add_instr(spec)),
        "ADC" => Some(Instr::AdcA {
            src: alu_src(operand(spec, 1)),
        }),
        "SUB" => Some(Instr::SubA {
            src: alu_src(operand(spec, 1)),
        }),
        "SBC" => Some(Instr::SbcA {
            src: alu_src(operand(spec, 1)),
        }),
        "AND" => Some(Instr::AndA {
            src: alu_src(operand(spec, 1)),
        }),
        "XOR" => Some(Instr::XorA {
            src: alu_src(operand(spec, 1)),
        }),
        "OR" => Some(Instr::OrA {
            src: alu_src(operand(spec, 1)),
        }),
        "CP" => Some(Instr::CpA {
            src: alu_src(operand(spec, 1)),
        }),
        "RLCA" => Some(Instr::Rlca),
        "RRCA" => Some(Instr::Rrca),
        "RLA" => Some(Instr::Rla),
        "RRA" => Some(Instr::Rra),
        "JP" => Some(jp_instr(spec)),
        "JR" => Some(jr_instr(spec)),
        "CALL" => Some(call_instr(spec)),
        "RET" => Some(ret_instr(spec)),
        "RETI" => Some(Instr::Reti),
        "RST" => Some(Instr::Rst {
            vector: rst_vector(operand(spec, 0).name.as_str()),
        }),
        "PUSH" => Some(Instr::Push {
            src: reg16_stack(operand(spec, 0).name.as_str()),
        }),
        "POP" => Some(Instr::Pop {
            dst: reg16_stack(operand(spec, 0).name.as_str()),
        }),
        "PREFIX" => None,
        mnemonic if mnemonic.starts_with("ILLEGAL_") => None,
        _ => panic!("unsupported gbdev opcode: {spec:?}"),
    }
}

fn instr_from_cbprefixed(spec: &GbdevOpcode) -> Instr {
    match spec.mnemonic.as_str() {
        "RLC" => Instr::Rlc {
            target: cb_target(operand(spec, 0)),
        },
        "RRC" => Instr::Rrc {
            target: cb_target(operand(spec, 0)),
        },
        "RL" => Instr::Rl {
            target: cb_target(operand(spec, 0)),
        },
        "RR" => Instr::Rr {
            target: cb_target(operand(spec, 0)),
        },
        "SLA" => Instr::Sla {
            target: cb_target(operand(spec, 0)),
        },
        "SRA" => Instr::Sra {
            target: cb_target(operand(spec, 0)),
        },
        "SWAP" => Instr::Swap {
            target: cb_target(operand(spec, 0)),
        },
        "SRL" => Instr::Srl {
            target: cb_target(operand(spec, 0)),
        },
        "BIT" => Instr::Bit {
            bit: bit_index(operand(spec, 0).name.as_str()),
            target: cb_target(operand(spec, 1)),
        },
        "RES" => Instr::Res {
            bit: bit_index(operand(spec, 0).name.as_str()),
            target: cb_target(operand(spec, 1)),
        },
        "SET" => Instr::Set {
            bit: bit_index(operand(spec, 0).name.as_str()),
            target: cb_target(operand(spec, 1)),
        },
        _ => panic!("unsupported gbdev CB opcode: {spec:?}"),
    }
}

fn ld_instr(opcode: u8, spec: &GbdevOpcode) -> Instr {
    let dst = operand(spec, 0);
    let src = operand(spec, 1);
    if dst.name == "A" && is_reg16_addr_src(opcode, src) {
        return Instr::LdAFromReg16Addr {
            src: reg16_addr(opcode, src.name.as_str()),
        };
    }
    if is_reg16_addr_dst(opcode, dst) && src.name == "A" {
        return Instr::LdReg16AddrFromA {
            dst: reg16_addr(opcode, dst.name.as_str()),
        };
    }
    if dst.name == "A" && src.name == "a16" && !src.immediate {
        return Instr::LdAFromDirect { addr: direct() };
    }
    if dst.name == "a16" && !dst.immediate && src.name == "A" {
        return Instr::LdDirectFromA { addr: direct() };
    }
    if dst.name == "a16" && !dst.immediate && src.name == "SP" {
        return Instr::LdDirectFromSp { addr: SAMPLE_U16 };
    }
    if dst.name == "A" && src.name == "a8" && !src.immediate {
        return Instr::LdAFromHighDirect {
            offset: HighDirectOffset::new(SAMPLE_A8),
        };
    }
    if dst.name == "a8" && !dst.immediate && src.name == "A" {
        return Instr::LdHighDirectFromA {
            offset: HighDirectOffset::new(SAMPLE_A8),
        };
    }
    if dst.name == "A" && src.name == "C" && !src.immediate {
        return Instr::LdAFromHighC;
    }
    if dst.name == "C" && !dst.immediate && src.name == "A" {
        return Instr::LdHighCFromA;
    }
    if dst.name == "SP" && src.name == "HL" {
        return Instr::LdSpFromHl;
    }
    if dst.name == "HL" && src.name == "SP" && operand(spec, 2).name == "e8" {
        return Instr::LdHlFromSpPlus { off: SAMPLE_I8 };
    }
    if let Some(dst) = reg16_data_opt(dst.name.as_str())
        && src.name == "n16"
    {
        return Instr::Ld16Imm {
            dst,
            imm: SAMPLE_U16,
        };
    }
    if let Some(dst) = reg8_opt(dst.name.as_str()) {
        if src.name == "n8" {
            return Instr::Ld8RegFromImm {
                dst,
                imm: SAMPLE_N8,
            };
        }
        if src.name == "HL" && !src.immediate {
            return Instr::Ld8RegFromHl { dst };
        }
        if let Some(src) = reg8_opt(src.name.as_str()) {
            return Instr::Ld8Reg { dst, src };
        }
    }
    if dst.name == "HL" && !dst.immediate {
        if src.name == "n8" {
            return Instr::Ld8HlFromImm { imm: SAMPLE_N8 };
        }
        if let Some(src) = reg8_opt(src.name.as_str()) {
            return Instr::Ld8HlFromReg { src };
        }
    }
    panic!("unsupported gbdev LD opcode: {spec:?}");
}

fn inc_instr(spec: &GbdevOpcode) -> Instr {
    let dst = operand(spec, 0);
    if let Some(dst) = reg8_opt(dst.name.as_str()) {
        return Instr::Inc8 {
            dst: IncDec8Target::Reg(dst),
        };
    }
    if dst.name == "HL" && !dst.immediate {
        return Instr::Inc8 {
            dst: IncDec8Target::HlIndirect,
        };
    }
    Instr::Inc16 {
        dst: reg16_data(dst.name.as_str()),
    }
}

fn dec_instr(spec: &GbdevOpcode) -> Instr {
    let dst = operand(spec, 0);
    if let Some(dst) = reg8_opt(dst.name.as_str()) {
        return Instr::Dec8 {
            dst: IncDec8Target::Reg(dst),
        };
    }
    if dst.name == "HL" && !dst.immediate {
        return Instr::Dec8 {
            dst: IncDec8Target::HlIndirect,
        };
    }
    Instr::Dec16 {
        dst: reg16_data(dst.name.as_str()),
    }
}

fn add_instr(spec: &GbdevOpcode) -> Instr {
    let lhs = operand(spec, 0);
    let rhs = operand(spec, 1);
    match lhs.name.as_str() {
        "A" => Instr::AddA { src: alu_src(rhs) },
        "HL" => Instr::AddHl {
            src: reg16_data(rhs.name.as_str()),
        },
        "SP" => Instr::AddSp { off: SAMPLE_I8 },
        _ => panic!("unsupported gbdev ADD opcode: {spec:?}"),
    }
}

fn jp_instr(spec: &GbdevOpcode) -> Instr {
    match spec.operands.as_slice() {
        [target] if target.name == "a16" => Instr::JpAbs {
            cond: None,
            addr: SAMPLE_U16,
        },
        [target] if target.name == "HL" => Instr::JpHl,
        [cond, target] if target.name == "a16" => Instr::JpAbs {
            cond: Some(condition(cond.name.as_str())),
            addr: SAMPLE_U16,
        },
        _ => panic!("unsupported gbdev JP opcode: {spec:?}"),
    }
}

fn jr_instr(spec: &GbdevOpcode) -> Instr {
    match spec.operands.as_slice() {
        [target] if target.name == "e8" => Instr::JrRel {
            cond: None,
            off: SAMPLE_I8,
        },
        [cond, target] if target.name == "e8" => Instr::JrRel {
            cond: Some(condition(cond.name.as_str())),
            off: SAMPLE_I8,
        },
        _ => panic!("unsupported gbdev JR opcode: {spec:?}"),
    }
}

fn call_instr(spec: &GbdevOpcode) -> Instr {
    match spec.operands.as_slice() {
        [target] if target.name == "a16" => Instr::Call {
            cond: None,
            addr: SAMPLE_U16,
        },
        [cond, target] if target.name == "a16" => Instr::Call {
            cond: Some(condition(cond.name.as_str())),
            addr: SAMPLE_U16,
        },
        _ => panic!("unsupported gbdev CALL opcode: {spec:?}"),
    }
}

fn ret_instr(spec: &GbdevOpcode) -> Instr {
    match spec.operands.as_slice() {
        [] => Instr::Ret { cond: None },
        [cond] => Instr::Ret {
            cond: Some(condition(cond.name.as_str())),
        },
        _ => panic!("unsupported gbdev RET opcode: {spec:?}"),
    }
}

fn alu_src(operand: &GbdevOperand) -> AluSrc8 {
    if operand.name == "n8" {
        return AluSrc8::Imm(SAMPLE_N8);
    }
    if operand.name == "HL" && !operand.immediate {
        return AluSrc8::HlIndirect;
    }
    AluSrc8::Reg(reg8(operand.name.as_str()))
}

fn cb_target(operand: &GbdevOperand) -> CbTarget {
    if operand.name == "HL" && !operand.immediate {
        CbTarget::HlIndirect
    } else {
        CbTarget::Reg(reg8(operand.name.as_str()))
    }
}

fn is_reg16_addr_src(opcode: u8, operand: &GbdevOperand) -> bool {
    !operand.immediate
        && matches!(
            (operand.name.as_str(), opcode),
            ("BC", _) | ("DE", _) | ("HL", 0x2A | 0x3A)
        )
}

fn is_reg16_addr_dst(opcode: u8, operand: &GbdevOperand) -> bool {
    !operand.immediate
        && matches!(
            (operand.name.as_str(), opcode),
            ("BC", _) | ("DE", _) | ("HL", 0x22 | 0x32)
        )
}

fn reg16_addr(opcode: u8, name: &str) -> Reg16Addr {
    match (name, opcode) {
        ("BC", _) => Reg16Addr::BC,
        ("DE", _) => Reg16Addr::DE,
        ("HL", 0x22 | 0x2A) => Reg16Addr::Hli,
        ("HL", 0x32 | 0x3A) => Reg16Addr::Hld,
        _ => panic!("unsupported gbdev register-address operand {name} at 0x{opcode:02X}"),
    }
}

fn reg8(name: &str) -> Reg8 {
    reg8_opt(name).unwrap_or_else(|| panic!("unsupported gbdev r8 operand {name}"))
}

fn reg8_opt(name: &str) -> Option<Reg8> {
    match name {
        "A" => Some(Reg8::A),
        "B" => Some(Reg8::B),
        "C" => Some(Reg8::C),
        "D" => Some(Reg8::D),
        "E" => Some(Reg8::E),
        "H" => Some(Reg8::H),
        "L" => Some(Reg8::L),
        _ => None,
    }
}

fn reg16_data(name: &str) -> Reg16Data {
    reg16_data_opt(name).unwrap_or_else(|| panic!("unsupported gbdev r16 operand {name}"))
}

fn reg16_data_opt(name: &str) -> Option<Reg16Data> {
    match name {
        "BC" => Some(Reg16Data::BC),
        "DE" => Some(Reg16Data::DE),
        "HL" => Some(Reg16Data::HL),
        "SP" => Some(Reg16Data::SP),
        _ => None,
    }
}

fn reg16_stack(name: &str) -> Reg16Stack {
    match name {
        "BC" => Reg16Stack::BC,
        "DE" => Reg16Stack::DE,
        "HL" => Reg16Stack::HL,
        "AF" => Reg16Stack::AF,
        _ => panic!("unsupported gbdev r16stk operand {name}"),
    }
}

fn condition(name: &str) -> Cond {
    match name {
        "NZ" => Cond::NZ,
        "Z" => Cond::Z,
        "NC" => Cond::NC,
        "C" => Cond::C,
        _ => panic!("unsupported gbdev condition {name}"),
    }
}

fn rst_vector(name: &str) -> RstVector {
    match name {
        "$00" => RstVector::V00,
        "$08" => RstVector::V08,
        "$10" => RstVector::V10,
        "$18" => RstVector::V18,
        "$20" => RstVector::V20,
        "$28" => RstVector::V28,
        "$30" => RstVector::V30,
        "$38" => RstVector::V38,
        _ => panic!("unsupported gbdev RST vector {name}"),
    }
}

fn bit_index(name: &str) -> BitIndex {
    let bit = name
        .parse::<u8>()
        .unwrap_or_else(|_| panic!("unsupported gbdev bit index {name}"));
    BitIndex::new(bit).unwrap_or_else(|| panic!("gbdev bit index out of range {name}"))
}

fn operand(spec: &GbdevOpcode, index: usize) -> &GbdevOperand {
    spec.operands
        .get(index)
        .unwrap_or_else(|| panic!("missing gbdev operand {index} in {spec:?}"))
}

fn direct() -> DirectAddr {
    DirectAddr::new(SAMPLE_U16).expect("sample address is below high memory")
}

fn parse_opcode_key(key: &str) -> u8 {
    u8::from_str_radix(
        key.strip_prefix("0x")
            .unwrap_or_else(|| panic!("gbdev opcode key must be 0x-prefixed: {key}")),
        16,
    )
    .unwrap_or_else(|_| panic!("gbdev opcode key must be hex: {key}"))
}

fn nonzero(value: u8) -> NonZeroU8 {
    NonZeroU8::new(value).unwrap_or_else(|| panic!("gbdev cycle cost must be nonzero"))
}
