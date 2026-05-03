//! Registry-driven PC and memory-access trap dispatcher.

use core::fmt;
use std::error::Error;

use serde::{Deserialize, Serialize};

use crate::primitives::{ClockCycles, EmuError, Regs, TrapPredicateError};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BreakpointId(pub u32);

impl BreakpointId {
    pub const RUN_UNTIL_PC: Self = Self(u32::MAX);
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TrapKind {
    Pc { addr: u16 },
    MemRead { range: AddressRange },
    MemWrite { range: AddressRange },
    MemRw { range: AddressRange },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize)]
#[serde(into = "AddressRangeRepr")]
pub struct AddressRange {
    start: u16,
    end_inclusive: u16,
}

impl AddressRange {
    pub fn new(start: u16, end_inclusive: u16) -> Result<Self, AddressRangeError> {
        if start > end_inclusive {
            return Err(AddressRangeError::StartAfterEnd {
                start,
                end_inclusive,
            });
        }
        Ok(Self {
            start,
            end_inclusive,
        })
    }

    #[must_use]
    pub const fn start(self) -> u16 {
        self.start
    }

    #[must_use]
    pub const fn end_inclusive(self) -> u16 {
        self.end_inclusive
    }

    #[must_use]
    pub const fn contains(self, addr: u16) -> bool {
        self.start <= addr && addr <= self.end_inclusive
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
struct AddressRangeRepr {
    start: u16,
    end_inclusive: u16,
}

impl From<AddressRange> for AddressRangeRepr {
    fn from(value: AddressRange) -> Self {
        Self {
            start: value.start,
            end_inclusive: value.end_inclusive,
        }
    }
}

impl TryFrom<AddressRangeRepr> for AddressRange {
    type Error = AddressRangeError;

    fn try_from(value: AddressRangeRepr) -> Result<Self, Self::Error> {
        Self::new(value.start, value.end_inclusive)
    }
}

impl<'de> Deserialize<'de> for AddressRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let repr = AddressRangeRepr::deserialize(deserializer)?;
        Self::try_from(repr).map_err(serde::de::Error::custom)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum AddressRangeError {
    StartAfterEnd { start: u16, end_inclusive: u16 },
}

impl fmt::Display for AddressRangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartAfterEnd {
                start,
                end_inclusive,
            } => write!(
                f,
                "address range start {start:#06x} is after end {end_inclusive:#06x}"
            ),
        }
    }
}

impl Error for AddressRangeError {}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TrapAction {
    HaltAndReport,
    Continue,
}

pub type TrapPredicate = dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static;

pub enum Predicate {
    Always,
    Closure(Box<TrapPredicate>),
    Source(String),
}

impl fmt::Debug for Predicate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Always => f.write_str("Always"),
            Self::Closure(_) => f.write_str("Closure(..)"),
            Self::Source(source) => f.debug_tuple("Source").field(source).finish(),
        }
    }
}

pub struct TrapContext<'a> {
    pub regs: Regs,
    pub pc: u16,
    pub access: Option<MemoryAccess>,
    pub cycle: ClockCycles,
    pub view: EmuReadOnlyView<'a>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MemoryAccess {
    pub addr: u16,
    pub value: u8,
    pub kind: MemoryAccessKind,
    pub cycle: ClockCycles,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MemoryAccessKind {
    InstrFetch,
    DataRead,
    Write,
}

pub trait EmuReadOnlyMemory {
    fn peek(&self, addr: u16) -> Result<u8, EmuError>;

    fn peek_range(&self, start: u16, len: usize) -> Result<Vec<u8>, EmuError> {
        (0..len)
            .map(|offset| {
                let offset = u16::try_from(offset).map_err(|_| EmuError::MemoryAccess {
                    addr: start,
                    reason: format!("range of length {len} exceeds u16 address space"),
                })?;
                let addr = start
                    .checked_add(offset)
                    .ok_or_else(|| EmuError::MemoryAccess {
                        addr: start,
                        reason: format!("range of length {len} overflows u16 address space"),
                    })?;
                self.peek(addr)
            })
            .collect()
    }
}

#[derive(Copy, Clone)]
pub struct EmuReadOnlyView<'a> {
    memory: &'a dyn EmuReadOnlyMemory,
    regs: Regs,
}

impl<'a> EmuReadOnlyView<'a> {
    #[must_use]
    pub const fn new(memory: &'a dyn EmuReadOnlyMemory, regs: Regs) -> Self {
        Self { memory, regs }
    }

    pub fn peek(&self, addr: u16) -> Result<u8, EmuError> {
        self.memory.peek(addr)
    }

    pub fn peek_range(&self, start: u16, len: usize) -> Result<Vec<u8>, EmuError> {
        self.memory.peek_range(start, len)
    }

    #[must_use]
    pub const fn regs(&self) -> Regs {
        self.regs
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum PredicateSpec {
    Always,
    Source(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrapSpec {
    pub id: BreakpointId,
    pub kind: TrapKind,
    pub action: TrapAction,
    pub predicate: PredicateSpec,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovedTrap {
    pub id: BreakpointId,
    pub kind: TrapKind,
    pub action: TrapAction,
    pub persistable_predicate: Option<PredicateSpec>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct TrapListEntry<'a> {
    pub id: BreakpointId,
    pub kind: &'a TrapKind,
    pub action: &'a TrapAction,
    pub persistable_predicate: Option<&'a str>,
}

#[derive(Debug)]
struct TrapEntry {
    id: BreakpointId,
    kind: TrapKind,
    action: TrapAction,
    predicate: Predicate,
}

#[derive(Debug, Default)]
pub struct TrapDispatcher {
    next_id: u32,
    entries: Vec<TrapEntry>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub(crate) struct TrapDispatchHit {
    pub id: BreakpointId,
    pub kind: TrapKind,
    pub action: TrapAction,
    pub cycle: ClockCycles,
}

impl TrapDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_pc(&mut self, addr: u16, predicate: Predicate, action: TrapAction) -> BreakpointId {
        self.add(TrapKind::Pc { addr }, predicate, action)
    }

    pub fn add_mem_read(
        &mut self,
        range: AddressRange,
        predicate: Predicate,
        action: TrapAction,
    ) -> BreakpointId {
        self.add(TrapKind::MemRead { range }, predicate, action)
    }

    pub fn add_mem_write(
        &mut self,
        range: AddressRange,
        predicate: Predicate,
        action: TrapAction,
    ) -> BreakpointId {
        self.add(TrapKind::MemWrite { range }, predicate, action)
    }

    pub fn add_mem_rw(
        &mut self,
        range: AddressRange,
        predicate: Predicate,
        action: TrapAction,
    ) -> BreakpointId {
        self.add(TrapKind::MemRw { range }, predicate, action)
    }

    fn add(&mut self, kind: TrapKind, predicate: Predicate, action: TrapAction) -> BreakpointId {
        let id = BreakpointId(self.next_id);
        self.next_id = self.next_id.saturating_add(1);
        self.entries.push(TrapEntry {
            id,
            kind,
            action,
            predicate,
        });
        id
    }

    pub fn remove(&mut self, id: BreakpointId) -> bool {
        let old_len = self.entries.len();
        self.entries.retain(|entry| entry.id != id);
        self.entries.len() != old_len
    }

    pub fn remove_entry(&mut self, id: BreakpointId) -> Option<RemovedTrap> {
        let index = self.entries.iter().position(|entry| entry.id == id)?;
        let entry = self.entries.remove(index);
        Some(RemovedTrap {
            id: entry.id,
            kind: entry.kind,
            action: entry.action,
            persistable_predicate: predicate_spec(&entry.predicate),
        })
    }

    pub fn list(&self) -> impl Iterator<Item = TrapListEntry<'_>> {
        self.entries.iter().map(|entry| TrapListEntry {
            id: entry.id,
            kind: &entry.kind,
            action: &entry.action,
            persistable_predicate: match &entry.predicate {
                Predicate::Source(source) => Some(source.as_str()),
                Predicate::Always | Predicate::Closure(_) => None,
            },
        })
    }

    pub fn export_persistable_specs(&self) -> Result<Vec<TrapSpec>, TrapPersistenceError> {
        self.entries
            .iter()
            .map(|entry| {
                let predicate = predicate_spec(&entry.predicate)
                    .ok_or(TrapPersistenceError::ClosureOnly { id: entry.id })?;
                Ok(TrapSpec {
                    id: entry.id,
                    kind: entry.kind,
                    action: entry.action,
                    predicate,
                })
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn has_pc_traps(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| matches!(entry.kind, TrapKind::Pc { .. }))
    }

    #[must_use]
    pub fn has_memory_traps(&self) -> bool {
        self.memory_trap_count() != 0
    }

    #[must_use]
    pub(crate) fn memory_trap_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| !matches!(entry.kind, TrapKind::Pc { .. }))
            .count()
    }

    pub(crate) fn dispatch_pc(
        &mut self,
        regs: Regs,
        cycle: ClockCycles,
        view: &dyn EmuReadOnlyMemory,
    ) -> Result<Vec<TrapDispatchHit>, TrapPredicateError> {
        let pc = regs.pc;
        let mut hits = Vec::new();
        for entry in &mut self.entries {
            if !matches!(entry.kind, TrapKind::Pc { addr } if addr == pc) {
                continue;
            }
            let context = TrapContext {
                regs,
                pc,
                access: None,
                cycle,
                view: EmuReadOnlyView::new(view, regs),
            };
            if predicate_matches(&mut entry.predicate, &context)? {
                let hit = TrapDispatchHit {
                    id: entry.id,
                    kind: entry.kind,
                    action: entry.action,
                    cycle,
                };
                hits.push(hit);
                if hit.action == TrapAction::HaltAndReport {
                    return Ok(hits);
                }
            }
        }
        Ok(hits)
    }

    pub(crate) fn dispatch_memory(
        &mut self,
        regs: Regs,
        accesses: &[MemoryAccess],
        view: &dyn EmuReadOnlyMemory,
    ) -> Result<Vec<TrapDispatchHit>, TrapPredicateError> {
        let mut hits = Vec::new();
        for access in accesses {
            for entry in &mut self.entries {
                if !memory_kind_matches(entry.kind, *access) {
                    continue;
                }
                let context = TrapContext {
                    regs,
                    pc: regs.pc,
                    access: Some(*access),
                    cycle: access.cycle,
                    view: EmuReadOnlyView::new(view, regs),
                };
                if predicate_matches(&mut entry.predicate, &context)? {
                    let hit = TrapDispatchHit {
                        id: entry.id,
                        kind: entry.kind,
                        action: entry.action,
                        cycle: access.cycle,
                    };
                    hits.push(hit);
                    if hit.action == TrapAction::HaltAndReport {
                        return Ok(hits);
                    }
                }
            }
        }
        Ok(hits)
    }
}

fn memory_kind_matches(kind: TrapKind, access: MemoryAccess) -> bool {
    match kind {
        TrapKind::MemRead { range } => {
            matches!(
                access.kind,
                MemoryAccessKind::InstrFetch | MemoryAccessKind::DataRead
            ) && range.contains(access.addr)
        }
        TrapKind::MemWrite { range } => {
            access.kind == MemoryAccessKind::Write && range.contains(access.addr)
        }
        TrapKind::MemRw { range } => range.contains(access.addr),
        TrapKind::Pc { .. } => false,
    }
}

fn predicate_matches(
    predicate: &mut Predicate,
    context: &TrapContext<'_>,
) -> Result<bool, TrapPredicateError> {
    match predicate {
        Predicate::Always => Ok(true),
        Predicate::Closure(callback) => callback(context),
        Predicate::Source(_) => Err(TrapPredicateError::SourceRequiresEvaluator),
    }
}

fn predicate_spec(predicate: &Predicate) -> Option<PredicateSpec> {
    match predicate {
        Predicate::Always => Some(PredicateSpec::Always),
        Predicate::Closure(_) => None,
        Predicate::Source(source) => Some(PredicateSpec::Source(source.clone())),
    }
}

impl From<TrapPredicateError> for EmuError {
    fn from(value: TrapPredicateError) -> Self {
        EmuError::TrapPredicate(value)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum TrapPersistenceError {
    ClosureOnly { id: BreakpointId },
}

impl fmt::Display for TrapPersistenceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClosureOnly { id } => {
                write!(f, "trap {:?} has a closure-only predicate", id)
            }
        }
    }
}

impl Error for TrapPersistenceError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyMemory;

    impl EmuReadOnlyMemory for EmptyMemory {
        fn peek(&self, _addr: u16) -> Result<u8, EmuError> {
            Ok(0)
        }
    }

    fn regs(pc: u16) -> Regs {
        Regs {
            a: 0,
            f: crate::Flags::new(0),
            b: 0,
            c: 0,
            d: 0,
            e: 0,
            h: 0,
            l: 0,
            sp: 0xFFFE,
            pc,
            ime: crate::ImeSnapshot::Disabled,
        }
    }

    #[test]
    fn address_range_rejects_inverted() {
        assert_eq!(
            AddressRange::new(0x10, 0x00),
            Err(AddressRangeError::StartAfterEnd {
                start: 0x10,
                end_inclusive: 0x00
            })
        );
    }

    #[test]
    fn address_range_deserialize_revalidates() {
        let bad = r#"{"start":16,"end_inclusive":0}"#;
        assert!(serde_json::from_str::<AddressRange>(bad).is_err());
    }

    #[test]
    fn pc_breakpoint_dispatches_before_instruction() {
        let mut traps = TrapDispatcher::new();
        let id = traps.add_pc(0x0150, Predicate::Always, TrapAction::HaltAndReport);

        let hit = traps
            .dispatch_pc(regs(0x0150), ClockCycles(10), &EmptyMemory)
            .expect("predicate ok")
            .pop()
            .expect("trap hit");

        assert_eq!(hit.id, id);
        assert_eq!(hit.kind, TrapKind::Pc { addr: 0x0150 });
    }

    #[test]
    fn export_persistable_specs_refuses_closures() {
        let mut traps = TrapDispatcher::new();
        let id = traps.add_pc(
            0x0100,
            Predicate::Closure(Box::new(|_| Ok(true))),
            TrapAction::HaltAndReport,
        );

        assert_eq!(
            traps.export_persistable_specs(),
            Err(TrapPersistenceError::ClosureOnly { id })
        );
    }

    #[test]
    fn source_predicate_returns_typed_error() {
        let mut traps = TrapDispatcher::new();
        traps.add_pc(
            0x0100,
            Predicate::Source("pc == 0x100".to_owned()),
            TrapAction::HaltAndReport,
        );

        assert_eq!(
            traps.dispatch_pc(regs(0x0100), ClockCycles(0), &EmptyMemory),
            Err(TrapPredicateError::SourceRequiresEvaluator)
        );
    }
}
