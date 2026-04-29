//! Machine-effect and privilege classification for LR35902 assembly IR.
//!
//! The classifier is intentionally instruction-local: it records what can be
//! known from the typed instruction alone. Register-indirect memory operations
//! keep an explicit dynamic-address effect so later reachability passes can use
//! provenance and section context instead of silently guessing a residency.

use serde::{Deserialize, Serialize};

use crate::isa::{
    AluSrc8, CbTarget, DirectAddr, HighDirectOffset, IncDec8Target, Instr, Reg16Addr, RstVector,
};
use crate::section::{LegalizationOp, PreLayoutOp};

/// Memory region reached through a concrete immediate address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum StaticMemoryRegion {
    Bank0,
    SwitchableRom,
    Vram,
    SwitchableSram,
    Wram,
    Oam,
    Hram,
    Io,
    Unusable,
}

/// Dynamic address source when an instruction does not carry a concrete address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DynamicAddress {
    Bc,
    De,
    Hl,
    HlIncrement,
    HlDecrement,
}

impl From<Reg16Addr> for DynamicAddress {
    fn from(value: Reg16Addr) -> Self {
        match value {
            Reg16Addr::BC => Self::Bc,
            Reg16Addr::DE => Self::De,
            Reg16Addr::Hli => Self::HlIncrement,
            Reg16Addr::Hld => Self::HlDecrement,
        }
    }
}

/// High-memory IO register addressed either statically or through `$FF00 + C`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum IoRegister {
    Address(u16),
    HighC,
}

/// MBC register class selected by a store into the cartridge register window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MbcRegisterClass {
    RamEnable,
    RomBankLow,
    RomBankHigh,
    SramBank,
    ModeSelect,
}

impl MbcRegisterClass {
    #[must_use]
    pub const fn for_addr(addr: u16) -> Option<Self> {
        match addr {
            0x0000..=0x1FFF => Some(Self::RamEnable),
            0x2000..=0x2FFF => Some(Self::RomBankLow),
            0x3000..=0x3FFF => Some(Self::RomBankHigh),
            0x4000..=0x5FFF => Some(Self::SramBank),
            0x6000..=0x7FFF => Some(Self::ModeSelect),
            _ => None,
        }
    }
}

/// Interrupt-control opcodes with scheduling-sensitive behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum InterruptControlOp {
    Halt,
    Stop,
    DisableInterrupts,
    EnableInterrupts,
}

/// Runtime structured op classes surfaced to reachability validation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SystemCallKind {
    BankLease,
    BankRelease,
    FarCall,
    Yield,
    TraceProbe,
    AssertBank,
}

/// Closed privilege lattice for instruction and structured op effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PrivilegeClass {
    Normal,
    Privileged,
    InterruptHandler,
}

/// Machine-visible effect of an instruction or structured op.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MachineEffect {
    PureCompute,
    LoadFromBank0,
    LoadFromSwitchableRom,
    LoadFromWram,
    LoadFromHram,
    LoadFromSwitchableSram,
    LoadFromVram,
    LoadFromOam,
    LoadFromIo {
        reg: IoRegister,
    },
    LoadFromUnusable,
    LoadFromDynamic {
        via: DynamicAddress,
    },
    LoadFromStack,
    StoreToWram,
    StoreToHram,
    StoreToSwitchableSram,
    StoreToVram,
    StoreToOam,
    StoreToIo {
        reg: IoRegister,
    },
    StoreToUnusable,
    StoreToDynamic {
        via: DynamicAddress,
    },
    StoreToStack,
    StoreToMixedStatic {
        first: StaticMemoryRegion,
        second: StaticMemoryRegion,
    },
    ReadModifyWriteDynamic {
        via: DynamicAddress,
    },
    StoreToMbcRegister {
        reg: MbcRegisterClass,
    },
    OpaqueBytes,
    InterruptControl(InterruptControlOp),
    UnconditionalBranch,
    ConditionalBranch,
    Call,
    Return,
    Reti,
    Rst {
        vector: RstVector,
    },
    SystemCall(SystemCallKind),
}

impl MachineEffect {
    #[must_use]
    pub const fn kind(self) -> MachineEffectKind {
        match self {
            Self::PureCompute => MachineEffectKind::PureCompute,
            Self::LoadFromBank0 => MachineEffectKind::LoadFromBank0,
            Self::LoadFromSwitchableRom => MachineEffectKind::LoadFromSwitchableRom,
            Self::LoadFromWram => MachineEffectKind::LoadFromWram,
            Self::LoadFromHram => MachineEffectKind::LoadFromHram,
            Self::LoadFromSwitchableSram => MachineEffectKind::LoadFromSwitchableSram,
            Self::LoadFromVram => MachineEffectKind::LoadFromVram,
            Self::LoadFromOam => MachineEffectKind::LoadFromOam,
            Self::LoadFromIo { .. } => MachineEffectKind::LoadFromIo,
            Self::LoadFromUnusable => MachineEffectKind::LoadFromUnusable,
            Self::LoadFromDynamic { .. } => MachineEffectKind::LoadFromDynamic,
            Self::LoadFromStack => MachineEffectKind::LoadFromStack,
            Self::StoreToWram => MachineEffectKind::StoreToWram,
            Self::StoreToHram => MachineEffectKind::StoreToHram,
            Self::StoreToSwitchableSram => MachineEffectKind::StoreToSwitchableSram,
            Self::StoreToVram => MachineEffectKind::StoreToVram,
            Self::StoreToOam => MachineEffectKind::StoreToOam,
            Self::StoreToIo { .. } => MachineEffectKind::StoreToIo,
            Self::StoreToUnusable => MachineEffectKind::StoreToUnusable,
            Self::StoreToDynamic { .. } => MachineEffectKind::StoreToDynamic,
            Self::StoreToStack => MachineEffectKind::StoreToStack,
            Self::StoreToMixedStatic { .. } => MachineEffectKind::StoreToMixedStatic,
            Self::ReadModifyWriteDynamic { .. } => MachineEffectKind::ReadModifyWriteDynamic,
            Self::StoreToMbcRegister { .. } => MachineEffectKind::StoreToMbcRegister,
            Self::OpaqueBytes => MachineEffectKind::OpaqueBytes,
            Self::InterruptControl(_) => MachineEffectKind::InterruptControl,
            Self::UnconditionalBranch => MachineEffectKind::UnconditionalBranch,
            Self::ConditionalBranch => MachineEffectKind::ConditionalBranch,
            Self::Call => MachineEffectKind::Call,
            Self::Return => MachineEffectKind::Return,
            Self::Reti => MachineEffectKind::Reti,
            Self::Rst { .. } => MachineEffectKind::Rst,
            Self::SystemCall(_) => MachineEffectKind::SystemCall,
        }
    }

    #[must_use]
    pub const fn disables_interrupts(self) -> bool {
        matches!(
            self,
            Self::InterruptControl(InterruptControlOp::DisableInterrupts)
        )
    }

    /// Returns true when instruction-local classification cannot prove the
    /// concrete address region and ReachabilityValidation must discharge it.
    #[must_use]
    pub const fn requires_dynamic_address_proof(self) -> bool {
        matches!(
            self,
            Self::LoadFromDynamic { .. }
                | Self::StoreToDynamic { .. }
                | Self::ReadModifyWriteDynamic { .. }
        )
    }
}

/// Parameter-free effect class used by section allowlists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum MachineEffectKind {
    PureCompute,
    LoadFromBank0,
    LoadFromSwitchableRom,
    LoadFromWram,
    LoadFromHram,
    LoadFromSwitchableSram,
    LoadFromVram,
    LoadFromOam,
    LoadFromIo,
    LoadFromUnusable,
    LoadFromDynamic,
    LoadFromStack,
    StoreToWram,
    StoreToHram,
    StoreToSwitchableSram,
    StoreToVram,
    StoreToOam,
    StoreToIo,
    StoreToUnusable,
    StoreToDynamic,
    StoreToStack,
    StoreToMixedStatic,
    ReadModifyWriteDynamic,
    StoreToMbcRegister,
    OpaqueBytes,
    InterruptControl,
    UnconditionalBranch,
    ConditionalBranch,
    Call,
    Return,
    Reti,
    Rst,
    SystemCall,
}

/// Classifies a typed instruction without using section or symbol context.
#[must_use]
pub const fn classify_effect(instr: &Instr) -> MachineEffect {
    match *instr {
        Instr::Nop
        | Instr::Ccf
        | Instr::Scf
        | Instr::Cpl
        | Instr::Daa
        | Instr::Ld8Reg { .. }
        | Instr::Ld8RegFromImm { .. }
        | Instr::Ld16Imm { .. }
        | Instr::LdSpFromHl
        | Instr::LdHlFromSpPlus { .. }
        | Instr::Inc16 { .. }
        | Instr::Dec16 { .. }
        | Instr::AddHl { .. }
        | Instr::AddSp { .. }
        | Instr::Rlca
        | Instr::Rrca
        | Instr::Rla
        | Instr::Rra => MachineEffect::PureCompute,
        Instr::Push { .. } => MachineEffect::StoreToStack,
        Instr::Pop { .. } => MachineEffect::LoadFromStack,
        Instr::Stop => MachineEffect::InterruptControl(InterruptControlOp::Stop),
        Instr::Halt => MachineEffect::InterruptControl(InterruptControlOp::Halt),
        Instr::Di => MachineEffect::InterruptControl(InterruptControlOp::DisableInterrupts),
        Instr::Ei => MachineEffect::InterruptControl(InterruptControlOp::EnableInterrupts),
        Instr::Ld8RegFromHl { .. } => MachineEffect::LoadFromDynamic {
            via: DynamicAddress::Hl,
        },
        Instr::Ld8HlFromReg { .. } | Instr::Ld8HlFromImm { .. } => MachineEffect::StoreToDynamic {
            via: DynamicAddress::Hl,
        },
        Instr::LdAFromReg16Addr { src } => MachineEffect::LoadFromDynamic {
            via: reg16_addr(src),
        },
        Instr::LdReg16AddrFromA { dst } => MachineEffect::StoreToDynamic {
            via: reg16_addr(dst),
        },
        Instr::LdAFromDirect { addr } => load_from_direct(addr),
        Instr::LdDirectFromA { addr } => store_to_direct(addr),
        Instr::LdAFromHighDirect { offset } => load_from_high_direct(offset),
        Instr::LdHighDirectFromA { offset } => store_to_high_direct(offset),
        Instr::LdAFromHighC => MachineEffect::LoadFromIo {
            reg: IoRegister::HighC,
        },
        Instr::LdHighCFromA => MachineEffect::StoreToIo {
            reg: IoRegister::HighC,
        },
        Instr::LdDirectFromSp { addr } => store_sp_to_addr(addr),
        Instr::AddA { src }
        | Instr::AdcA { src }
        | Instr::SubA { src }
        | Instr::SbcA { src }
        | Instr::AndA { src }
        | Instr::OrA { src }
        | Instr::XorA { src }
        | Instr::CpA { src } => alu_src_effect(src),
        Instr::Inc8 { dst } | Instr::Dec8 { dst } => inc_dec_effect(dst),
        Instr::Rlc { target }
        | Instr::Rrc { target }
        | Instr::Rl { target }
        | Instr::Rr { target }
        | Instr::Sla { target }
        | Instr::Sra { target }
        | Instr::Srl { target }
        | Instr::Swap { target }
        | Instr::Res { target, .. }
        | Instr::Set { target, .. } => cb_read_modify_write_effect(target),
        Instr::Bit { target, .. } => cb_read_effect(target),
        Instr::JpAbs { cond: None, .. } | Instr::JpHl => MachineEffect::UnconditionalBranch,
        Instr::JpAbs { cond: Some(_), .. } | Instr::JrRel { cond: Some(_), .. } => {
            MachineEffect::ConditionalBranch
        }
        Instr::JrRel { cond: None, .. } => MachineEffect::UnconditionalBranch,
        Instr::Call { .. } => MachineEffect::Call,
        Instr::Ret { .. } => MachineEffect::Return,
        Instr::Reti => MachineEffect::Reti,
        Instr::Rst { vector } => MachineEffect::Rst { vector },
    }
}

/// Classifies a pre-layout op for section-level validation.
#[must_use]
pub fn classify_pre_layout_op(op: &PreLayoutOp) -> MachineEffect {
    match op {
        PreLayoutOp::BankLease(_) => MachineEffect::SystemCall(SystemCallKind::BankLease),
        PreLayoutOp::BankRelease { .. } => MachineEffect::SystemCall(SystemCallKind::BankRelease),
        PreLayoutOp::Yield { .. } => MachineEffect::SystemCall(SystemCallKind::Yield),
        PreLayoutOp::TraceProbe { .. } => MachineEffect::SystemCall(SystemCallKind::TraceProbe),
        PreLayoutOp::AssertBank { .. } => MachineEffect::SystemCall(SystemCallKind::AssertBank),
    }
}

/// Classifies a legalization op for section-level validation.
#[must_use]
pub fn classify_legalization_op(op: &LegalizationOp) -> MachineEffect {
    match op {
        LegalizationOp::FarCall { .. } => MachineEffect::SystemCall(SystemCallKind::FarCall),
    }
}

/// Returns the privilege class required by an effect.
#[must_use]
pub const fn privilege_of(effect: &MachineEffect) -> PrivilegeClass {
    match *effect {
        MachineEffect::StoreToMbcRegister { .. } => PrivilegeClass::Privileged,
        MachineEffect::StoreToMixedStatic { first, second } => {
            if is_privileged_static_region(first) || is_privileged_static_region(second) {
                PrivilegeClass::Privileged
            } else {
                PrivilegeClass::Normal
            }
        }
        MachineEffect::OpaqueBytes
        | MachineEffect::InterruptControl(_)
        | MachineEffect::SystemCall(SystemCallKind::BankLease)
        | MachineEffect::SystemCall(SystemCallKind::BankRelease)
        | MachineEffect::SystemCall(SystemCallKind::AssertBank) => PrivilegeClass::Privileged,
        MachineEffect::Reti => PrivilegeClass::InterruptHandler,
        MachineEffect::PureCompute
        | MachineEffect::LoadFromBank0
        | MachineEffect::LoadFromSwitchableRom
        | MachineEffect::LoadFromWram
        | MachineEffect::LoadFromHram
        | MachineEffect::LoadFromSwitchableSram
        | MachineEffect::LoadFromVram
        | MachineEffect::LoadFromOam
        | MachineEffect::LoadFromIo { .. }
        | MachineEffect::LoadFromUnusable
        | MachineEffect::LoadFromDynamic { .. }
        | MachineEffect::LoadFromStack
        | MachineEffect::StoreToWram
        | MachineEffect::StoreToHram
        | MachineEffect::StoreToSwitchableSram
        | MachineEffect::StoreToVram
        | MachineEffect::StoreToOam
        | MachineEffect::StoreToIo { .. }
        | MachineEffect::StoreToUnusable
        | MachineEffect::StoreToDynamic { .. }
        | MachineEffect::StoreToStack
        | MachineEffect::ReadModifyWriteDynamic { .. }
        | MachineEffect::UnconditionalBranch
        | MachineEffect::ConditionalBranch
        | MachineEffect::Call
        | MachineEffect::Return
        | MachineEffect::Rst { .. }
        | MachineEffect::SystemCall(SystemCallKind::FarCall)
        | MachineEffect::SystemCall(SystemCallKind::Yield)
        | MachineEffect::SystemCall(SystemCallKind::TraceProbe) => PrivilegeClass::Normal,
    }
}

const fn reg16_addr(addr: Reg16Addr) -> DynamicAddress {
    match addr {
        Reg16Addr::BC => DynamicAddress::Bc,
        Reg16Addr::DE => DynamicAddress::De,
        Reg16Addr::Hli => DynamicAddress::HlIncrement,
        Reg16Addr::Hld => DynamicAddress::HlDecrement,
    }
}

const fn alu_src_effect(src: AluSrc8) -> MachineEffect {
    match src {
        AluSrc8::Reg(_) | AluSrc8::Imm(_) => MachineEffect::PureCompute,
        AluSrc8::HlIndirect => MachineEffect::LoadFromDynamic {
            via: DynamicAddress::Hl,
        },
    }
}

const fn inc_dec_effect(dst: IncDec8Target) -> MachineEffect {
    match dst {
        IncDec8Target::Reg(_) => MachineEffect::PureCompute,
        IncDec8Target::HlIndirect => MachineEffect::ReadModifyWriteDynamic {
            via: DynamicAddress::Hl,
        },
    }
}

const fn cb_read_effect(target: CbTarget) -> MachineEffect {
    match target {
        CbTarget::Reg(_) => MachineEffect::PureCompute,
        CbTarget::HlIndirect => MachineEffect::LoadFromDynamic {
            via: DynamicAddress::Hl,
        },
    }
}

const fn cb_read_modify_write_effect(target: CbTarget) -> MachineEffect {
    match target {
        CbTarget::Reg(_) => MachineEffect::PureCompute,
        CbTarget::HlIndirect => MachineEffect::ReadModifyWriteDynamic {
            via: DynamicAddress::Hl,
        },
    }
}

const fn load_from_direct(addr: DirectAddr) -> MachineEffect {
    match static_region(addr.get()) {
        StaticMemoryRegion::Bank0 => MachineEffect::LoadFromBank0,
        StaticMemoryRegion::SwitchableRom => MachineEffect::LoadFromSwitchableRom,
        StaticMemoryRegion::Vram => MachineEffect::LoadFromVram,
        StaticMemoryRegion::SwitchableSram => MachineEffect::LoadFromSwitchableSram,
        StaticMemoryRegion::Wram => MachineEffect::LoadFromWram,
        StaticMemoryRegion::Oam => MachineEffect::LoadFromOam,
        StaticMemoryRegion::Hram => MachineEffect::LoadFromHram,
        StaticMemoryRegion::Io => MachineEffect::LoadFromIo {
            reg: IoRegister::Address(addr.get()),
        },
        StaticMemoryRegion::Unusable => MachineEffect::LoadFromUnusable,
    }
}

const fn store_to_direct(addr: DirectAddr) -> MachineEffect {
    store_to_addr(addr.get())
}

const fn store_to_addr(addr: u16) -> MachineEffect {
    match static_region(addr) {
        StaticMemoryRegion::Bank0 | StaticMemoryRegion::SwitchableRom => {
            MachineEffect::StoreToMbcRegister {
                reg: match MbcRegisterClass::for_addr(addr) {
                    Some(reg) => reg,
                    None => MbcRegisterClass::ModeSelect,
                },
            }
        }
        StaticMemoryRegion::Vram => MachineEffect::StoreToVram,
        StaticMemoryRegion::SwitchableSram => MachineEffect::StoreToSwitchableSram,
        StaticMemoryRegion::Wram => MachineEffect::StoreToWram,
        StaticMemoryRegion::Oam => MachineEffect::StoreToOam,
        StaticMemoryRegion::Hram => MachineEffect::StoreToHram,
        StaticMemoryRegion::Io => MachineEffect::StoreToIo {
            reg: IoRegister::Address(addr),
        },
        StaticMemoryRegion::Unusable => MachineEffect::StoreToUnusable,
    }
}

const fn store_sp_to_addr(addr: u16) -> MachineEffect {
    let first = static_region(addr);
    let second = static_region(addr.wrapping_add(1));
    if same_static_region(first, second) {
        store_to_addr(addr)
    } else {
        MachineEffect::StoreToMixedStatic { first, second }
    }
}

const fn load_from_high_direct(offset: HighDirectOffset) -> MachineEffect {
    let addr = offset.absolute_addr();
    if is_hram_addr(addr) {
        MachineEffect::LoadFromHram
    } else {
        MachineEffect::LoadFromIo {
            reg: IoRegister::Address(addr),
        }
    }
}

const fn store_to_high_direct(offset: HighDirectOffset) -> MachineEffect {
    let addr = offset.absolute_addr();
    if is_hram_addr(addr) {
        MachineEffect::StoreToHram
    } else {
        MachineEffect::StoreToIo {
            reg: IoRegister::Address(addr),
        }
    }
}

const fn static_region(addr: u16) -> StaticMemoryRegion {
    match addr {
        0x0000..=0x3FFF => StaticMemoryRegion::Bank0,
        0x4000..=0x7FFF => StaticMemoryRegion::SwitchableRom,
        0x8000..=0x9FFF => StaticMemoryRegion::Vram,
        0xA000..=0xBFFF => StaticMemoryRegion::SwitchableSram,
        0xC000..=0xFDFF => StaticMemoryRegion::Wram,
        0xFE00..=0xFE9F => StaticMemoryRegion::Oam,
        0xFEA0..=0xFEFF => StaticMemoryRegion::Unusable,
        0xFF00..=0xFF7F | 0xFFFF => StaticMemoryRegion::Io,
        0xFF80..=0xFFFE => StaticMemoryRegion::Hram,
    }
}

const fn is_hram_addr(addr: u16) -> bool {
    matches!(addr, 0xFF80..=0xFFFE)
}

const fn same_static_region(left: StaticMemoryRegion, right: StaticMemoryRegion) -> bool {
    matches!(
        (left, right),
        (StaticMemoryRegion::Bank0, StaticMemoryRegion::Bank0)
            | (
                StaticMemoryRegion::SwitchableRom,
                StaticMemoryRegion::SwitchableRom
            )
            | (StaticMemoryRegion::Vram, StaticMemoryRegion::Vram)
            | (
                StaticMemoryRegion::SwitchableSram,
                StaticMemoryRegion::SwitchableSram
            )
            | (StaticMemoryRegion::Wram, StaticMemoryRegion::Wram)
            | (StaticMemoryRegion::Oam, StaticMemoryRegion::Oam)
            | (StaticMemoryRegion::Hram, StaticMemoryRegion::Hram)
            | (StaticMemoryRegion::Io, StaticMemoryRegion::Io)
            | (StaticMemoryRegion::Unusable, StaticMemoryRegion::Unusable)
    )
}

const fn is_privileged_static_region(region: StaticMemoryRegion) -> bool {
    matches!(
        region,
        StaticMemoryRegion::Bank0 | StaticMemoryRegion::SwitchableRom
    )
}

#[cfg(test)]
#[test]
fn classify_exhaustive() {
    use crate::isa::{BitIndex, Cond, Reg8, Reg16Addr, Reg16Data, Reg16Stack, RstVector};

    let direct_bank0 = DirectAddr::new(0x1234).expect("bank0");
    let direct_romx = DirectAddr::new(0x4000).expect("romx");
    let direct_vram = DirectAddr::new(0x8000).expect("vram");
    let direct_sram = DirectAddr::new(0xA000).expect("sram");
    let direct_wram = DirectAddr::new(0xC000).expect("wram");
    let direct_oam = DirectAddr::new(0xFE00).expect("oam");
    let direct_unusable = DirectAddr::new(0xFEA0).expect("unusable");
    let high_io = HighDirectOffset::new(0x40);
    let high_hram = HighDirectOffset::new(0x80);
    let bit = BitIndex::new(3).expect("bit");

    let cases = [
        (Instr::Nop, MachineEffectKind::PureCompute),
        (Instr::Stop, MachineEffectKind::InterruptControl),
        (Instr::Halt, MachineEffectKind::InterruptControl),
        (Instr::Di, MachineEffectKind::InterruptControl),
        (Instr::Ei, MachineEffectKind::InterruptControl),
        (Instr::Ccf, MachineEffectKind::PureCompute),
        (Instr::Scf, MachineEffectKind::PureCompute),
        (Instr::Cpl, MachineEffectKind::PureCompute),
        (Instr::Daa, MachineEffectKind::PureCompute),
        (
            Instr::Ld8Reg {
                dst: Reg8::A,
                src: Reg8::B,
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Ld8RegFromImm {
                dst: Reg8::A,
                imm: 1,
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Ld8RegFromHl { dst: Reg8::A },
            MachineEffectKind::LoadFromDynamic,
        ),
        (
            Instr::Ld8HlFromReg { src: Reg8::A },
            MachineEffectKind::StoreToDynamic,
        ),
        (
            Instr::Ld8HlFromImm { imm: 1 },
            MachineEffectKind::StoreToDynamic,
        ),
        (
            Instr::LdAFromReg16Addr { src: Reg16Addr::BC },
            MachineEffectKind::LoadFromDynamic,
        ),
        (
            Instr::LdReg16AddrFromA { dst: Reg16Addr::DE },
            MachineEffectKind::StoreToDynamic,
        ),
        (
            Instr::LdAFromDirect { addr: direct_bank0 },
            MachineEffectKind::LoadFromBank0,
        ),
        (
            Instr::LdAFromDirect { addr: direct_romx },
            MachineEffectKind::LoadFromSwitchableRom,
        ),
        (
            Instr::LdAFromDirect { addr: direct_vram },
            MachineEffectKind::LoadFromVram,
        ),
        (
            Instr::LdAFromDirect { addr: direct_sram },
            MachineEffectKind::LoadFromSwitchableSram,
        ),
        (
            Instr::LdAFromDirect { addr: direct_wram },
            MachineEffectKind::LoadFromWram,
        ),
        (
            Instr::LdAFromDirect { addr: direct_oam },
            MachineEffectKind::LoadFromOam,
        ),
        (
            Instr::LdAFromDirect {
                addr: direct_unusable,
            },
            MachineEffectKind::LoadFromUnusable,
        ),
        (
            Instr::LdDirectFromA { addr: direct_vram },
            MachineEffectKind::StoreToVram,
        ),
        (
            Instr::LdAFromHighDirect { offset: high_io },
            MachineEffectKind::LoadFromIo,
        ),
        (
            Instr::LdHighDirectFromA { offset: high_hram },
            MachineEffectKind::StoreToHram,
        ),
        (Instr::LdAFromHighC, MachineEffectKind::LoadFromIo),
        (Instr::LdHighCFromA, MachineEffectKind::StoreToIo),
        (
            Instr::Ld16Imm {
                dst: Reg16Data::HL,
                imm: 0x1234,
            },
            MachineEffectKind::PureCompute,
        ),
        (Instr::LdSpFromHl, MachineEffectKind::PureCompute),
        (
            Instr::LdDirectFromSp { addr: 0x2000 },
            MachineEffectKind::StoreToMbcRegister,
        ),
        (
            Instr::LdDirectFromSp { addr: 0xFF80 },
            MachineEffectKind::StoreToHram,
        ),
        (
            Instr::LdDirectFromSp { addr: 0x9FFF },
            MachineEffectKind::StoreToMixedStatic,
        ),
        (
            Instr::LdHlFromSpPlus { off: -1 },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::AddA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::AdcA {
                src: AluSrc8::HlIndirect,
            },
            MachineEffectKind::LoadFromDynamic,
        ),
        (
            Instr::SubA {
                src: AluSrc8::Reg(Reg8::B),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::SbcA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::AndA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::OrA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::XorA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::CpA {
                src: AluSrc8::Imm(1),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Inc8 {
                dst: IncDec8Target::HlIndirect,
            },
            MachineEffectKind::ReadModifyWriteDynamic,
        ),
        (
            Instr::Dec8 {
                dst: IncDec8Target::Reg(Reg8::C),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Inc16 { dst: Reg16Data::BC },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Dec16 { dst: Reg16Data::DE },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::AddHl { src: Reg16Data::SP },
            MachineEffectKind::PureCompute,
        ),
        (Instr::AddSp { off: 1 }, MachineEffectKind::PureCompute),
        (Instr::Rlca, MachineEffectKind::PureCompute),
        (Instr::Rrca, MachineEffectKind::PureCompute),
        (Instr::Rla, MachineEffectKind::PureCompute),
        (Instr::Rra, MachineEffectKind::PureCompute),
        (
            Instr::Rlc {
                target: CbTarget::HlIndirect,
            },
            MachineEffectKind::ReadModifyWriteDynamic,
        ),
        (
            Instr::Rrc {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Rl {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Rr {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Sla {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Sra {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Srl {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Swap {
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::Bit {
                bit,
                target: CbTarget::HlIndirect,
            },
            MachineEffectKind::LoadFromDynamic,
        ),
        (
            Instr::Res {
                bit,
                target: CbTarget::HlIndirect,
            },
            MachineEffectKind::ReadModifyWriteDynamic,
        ),
        (
            Instr::Set {
                bit,
                target: CbTarget::Reg(Reg8::A),
            },
            MachineEffectKind::PureCompute,
        ),
        (
            Instr::JpAbs {
                cond: None,
                addr: 0x1234,
            },
            MachineEffectKind::UnconditionalBranch,
        ),
        (
            Instr::JpAbs {
                cond: Some(Cond::C),
                addr: 0x1234,
            },
            MachineEffectKind::ConditionalBranch,
        ),
        (Instr::JpHl, MachineEffectKind::UnconditionalBranch),
        (
            Instr::JrRel { cond: None, off: 4 },
            MachineEffectKind::UnconditionalBranch,
        ),
        (
            Instr::JrRel {
                cond: Some(Cond::NZ),
                off: -2,
            },
            MachineEffectKind::ConditionalBranch,
        ),
        (
            Instr::Call {
                cond: Some(Cond::Z),
                addr: 0x1234,
            },
            MachineEffectKind::Call,
        ),
        (
            Instr::Call {
                cond: None,
                addr: 0x1234,
            },
            MachineEffectKind::Call,
        ),
        (Instr::Ret { cond: None }, MachineEffectKind::Return),
        (
            Instr::Ret {
                cond: Some(Cond::NC),
            },
            MachineEffectKind::Return,
        ),
        (Instr::Reti, MachineEffectKind::Reti),
        (
            Instr::Rst {
                vector: RstVector::V38,
            },
            MachineEffectKind::Rst,
        ),
        (
            Instr::Push {
                src: Reg16Stack::AF,
            },
            MachineEffectKind::StoreToStack,
        ),
        (
            Instr::Pop {
                dst: Reg16Stack::HL,
            },
            MachineEffectKind::LoadFromStack,
        ),
    ];

    for (instr, kind) in cases {
        assert_eq!(classify_effect(&instr).kind(), kind, "{instr:?}");
    }
}

#[cfg(test)]
#[test]
fn mbc_writes_are_privileged() {
    let cases = [
        (0x0000, MbcRegisterClass::RamEnable),
        (0x1FFF, MbcRegisterClass::RamEnable),
        (0x2000, MbcRegisterClass::RomBankLow),
        (0x2FFF, MbcRegisterClass::RomBankLow),
        (0x3000, MbcRegisterClass::RomBankHigh),
        (0x3FFF, MbcRegisterClass::RomBankHigh),
        (0x4000, MbcRegisterClass::SramBank),
        (0x5FFF, MbcRegisterClass::SramBank),
        (0x6000, MbcRegisterClass::ModeSelect),
        (0x7FFF, MbcRegisterClass::ModeSelect),
    ];

    for (addr, reg) in cases {
        let addr = DirectAddr::new(addr).expect("mbc register address");
        let effect = classify_effect(&Instr::LdDirectFromA { addr });
        assert_eq!(effect, MachineEffect::StoreToMbcRegister { reg });
        assert_eq!(privilege_of(&effect), PrivilegeClass::Privileged);
    }

    for kind in [
        SystemCallKind::BankLease,
        SystemCallKind::BankRelease,
        SystemCallKind::AssertBank,
    ] {
        assert_eq!(
            privilege_of(&MachineEffect::SystemCall(kind)),
            PrivilegeClass::Privileged
        );
    }
}

#[cfg(test)]
#[test]
fn dynamic_memory_effects_name_reachability_obligation() {
    let store = classify_effect(&Instr::Ld8HlFromReg {
        src: crate::isa::Reg8::A,
    });
    assert!(store.requires_dynamic_address_proof());
    assert_eq!(privilege_of(&store), PrivilegeClass::Normal);

    let direct = DirectAddr::new(0xC000).expect("wram address");
    let direct_store = classify_effect(&Instr::LdDirectFromA { addr: direct });
    assert!(!direct_store.requires_dynamic_address_proof());
}
