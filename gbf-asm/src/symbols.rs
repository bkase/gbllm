//! Canonical symbol names and post-layout symbol resolution.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::layout::{AddressSpace, BankIndex, LayoutPlan};
use crate::section::{SectionId, SectionRole};

/// Error returned when a symbol name is not canonical.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolNameError {
    Empty,
    EmptySegment,
    InvalidCharacter { ch: char },
}

impl fmt::Display for SymbolNameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("symbol name must not be empty"),
            Self::EmptySegment => f.write_str("symbol name must not contain empty segments"),
            Self::InvalidCharacter { ch } => {
                write!(f, "symbol name contains invalid character {ch:?}")
            }
        }
    }
}

impl std::error::Error for SymbolNameError {}

/// One canonical symbol segment.
///
/// A `SymbolName` may contain dots between segments; helper constructors accept
/// `SymbolSegment`s so caller-owned components cannot smuggle dots and collide
/// with a different conceptual path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SymbolSegment(Cow<'static, str>);

impl SymbolSegment {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Result<Self, SymbolNameError> {
        let value = value.into();
        validate_symbol_segment(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SymbolSegment {
    type Error = SymbolNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&'static str> for SymbolSegment {
    type Error = SymbolNameError;

    fn try_from(value: &'static str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SymbolSegment> for String {
    fn from(value: SymbolSegment) -> Self {
        value.0.into_owned()
    }
}

impl fmt::Display for SymbolSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Stable dot-separated symbol name.
///
/// Canonical segments use lowercase ASCII letters, digits, and `_`. This keeps
/// harness-facing names deterministic and shell/report friendly.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SymbolName(Cow<'static, str>);

impl SymbolName {
    pub fn new(value: impl Into<Cow<'static, str>>) -> Result<Self, SymbolNameError> {
        let value = value.into();
        validate_symbol_name(&value)?;
        Ok(Self(value))
    }

    pub fn kernel(family: &str, id: u32) -> Result<Self, SymbolNameError> {
        let family = SymbolSegment::new(family.to_owned())?;
        Self::new(format!("kernel.{family}.{id}"))
    }

    pub fn expert(layer: u32, id: u32) -> Result<Self, SymbolNameError> {
        Self::new(format!("expert.{layer}.{id}"))
    }

    pub fn runtime(module: &str, symbol: &str) -> Result<Self, SymbolNameError> {
        let module = SymbolSegment::new(module.to_owned())?;
        let symbol = SymbolSegment::new(symbol.to_owned())?;
        Self::new(format!("runtime.{module}.{symbol}"))
    }

    pub fn runtime_thunk_for(target: &SymbolName) -> Result<Self, SymbolNameError> {
        Self::new(format!("runtime.banking.thunk.{}", target.as_str()))
    }

    pub fn section(role: SectionRole, id: SectionId) -> Result<Self, SymbolNameError> {
        Self::new(format!("section.{}.{}", role.canonical_name(), id.get()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_cow(self) -> Cow<'static, str> {
        self.0
    }
}

impl TryFrom<String> for SymbolName {
    type Error = SymbolNameError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl TryFrom<&'static str> for SymbolName {
    type Error = SymbolNameError;

    fn try_from(value: &'static str) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<SymbolName> for String {
    fn from(value: SymbolName) -> Self {
        value.0.into_owned()
    }
}

impl AsRef<str> for SymbolName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl fmt::Display for SymbolName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn validate_symbol_name(value: &str) -> Result<(), SymbolNameError> {
    if value.is_empty() {
        return Err(SymbolNameError::Empty);
    }

    for segment in value.split('.') {
        validate_symbol_segment(segment)?;
    }

    Ok(())
}

fn validate_symbol_segment(value: &str) -> Result<(), SymbolNameError> {
    if value.is_empty() {
        return Err(SymbolNameError::EmptySegment);
    }
    for ch in value.chars() {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
            return Err(SymbolNameError::InvalidCharacter { ch });
        }
    }
    Ok(())
}

/// Resolved post-layout symbol address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SymbolAddress {
    pub section: SectionId,
    pub offset: u32,
}

impl SymbolAddress {
    #[must_use]
    pub const fn new(section: SectionId, offset: u32) -> Self {
        Self { section, offset }
    }
}

/// Errors raised by the symbol table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolTableError {
    DuplicateName {
        name: SymbolName,
        existing: SymbolAddress,
        new: SymbolAddress,
    },
}

impl fmt::Display for SymbolTableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName {
                name,
                existing,
                new,
            } => write!(
                f,
                "symbol {name} already resolves to {existing:?}, cannot also resolve to {new:?}"
            ),
        }
    }
}

impl std::error::Error for SymbolTableError {}

/// Deterministic post-layout symbol table.
///
/// Names are unique. Addresses may have multiple names so section starts,
/// entrypoints, and harness checkpoint labels can alias without inventing fake
/// offsets.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolTable {
    by_name: BTreeMap<SymbolName, SymbolAddress>,
}

impl SymbolTable {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(
        &mut self,
        name: SymbolName,
        address: SymbolAddress,
    ) -> Result<(), SymbolTableError> {
        if let Some(existing) = self.by_name.get(&name).copied() {
            return Err(SymbolTableError::DuplicateName {
                name,
                existing,
                new: address,
            });
        }
        self.by_name.insert(name, address);
        Ok(())
    }

    #[must_use]
    pub fn resolve(&self, name: &SymbolName) -> Option<SymbolAddress> {
        self.by_name.get(name).copied()
    }

    #[must_use]
    pub fn names_for(&self, address: SymbolAddress) -> Vec<&SymbolName> {
        self.by_name
            .iter()
            .filter_map(|(name, candidate)| (*candidate == address).then_some(name))
            .collect()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SymbolName, SymbolAddress)> {
        self.by_name.iter().map(|(name, address)| (name, *address))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymOptions {
    pub include_externals_as_comments: bool,
    pub rom_only: bool,
    pub dot_safe_separator: bool,
}

impl Default for SymOptions {
    fn default() -> Self {
        Self {
            include_externals_as_comments: true,
            rom_only: false,
            dot_safe_separator: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymError {
    MissingPlacement {
        section_id: SectionId,
    },
    AddressOverflow {
        section_id: SectionId,
        offset: u32,
    },
    DotSafeNameCollision {
        rewritten: String,
        originals: [SymbolName; 2],
    },
    Parse(String),
}

impl fmt::Display for SymError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPlacement { section_id } => {
                write!(
                    f,
                    "symbol table references unplaced section {}",
                    section_id.get()
                )
            }
            Self::AddressOverflow { section_id, offset } => write!(
                f,
                "symbol in section {} at offset {offset} overflows a 16-bit CPU address",
                section_id.get()
            ),
            Self::DotSafeNameCollision {
                rewritten,
                originals,
            } => write!(
                f,
                "dot-safe .sym name {rewritten} collides for {} and {}",
                originals[0], originals[1]
            ),
            Self::Parse(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SymError {}

/// One parsed RGBDS-compatible `.sym` line.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SymEntry {
    pub bank: Option<u16>,
    pub addr: u16,
    pub name: String,
}

impl fmt::Display for SymEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.bank {
            Some(bank) => write!(f, "{bank:02X}:{:04X} {}", self.addr, self.name),
            None => write!(f, "{:04X} {}", self.addr, self.name),
        }
    }
}

impl FromStr for SymEntry {
    type Err = SymError;

    fn from_str(line: &str) -> Result<Self, Self::Err> {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            return Err(SymError::Parse("empty/comment .sym line".to_owned()));
        }
        let (loc, name) = line
            .split_once(char::is_whitespace)
            .ok_or_else(|| SymError::Parse(format!("missing symbol name in {line:?}")))?;
        let name = name.trim();
        if name.is_empty() {
            return Err(SymError::Parse(format!("missing symbol name in {line:?}")));
        }
        if let Some((bank, addr)) = loc.split_once(':') {
            let bank = u16::from_str_radix(bank, 16)
                .map_err(|_| SymError::Parse(format!("invalid bank in {line:?}")))?;
            let addr = u16::from_str_radix(addr, 16)
                .map_err(|_| SymError::Parse(format!("invalid address in {line:?}")))?;
            Ok(Self {
                bank: Some(bank),
                addr,
                name: name.to_owned(),
            })
        } else {
            let addr = u16::from_str_radix(loc, 16)
                .map_err(|_| SymError::Parse(format!("invalid address in {line:?}")))?;
            Ok(Self {
                bank: None,
                addr,
                name: name.to_owned(),
            })
        }
    }
}

pub fn parse_sym_entries(input: &str) -> Result<Vec<SymEntry>, SymError> {
    input
        .lines()
        .filter(|line| {
            let line = line.trim();
            !line.is_empty() && !line.starts_with(';')
        })
        .map(str::parse)
        .collect()
}

/// Writes a deterministic RGBDS-compatible Game Boy `.sym` file.
pub fn write_sym(
    layout: &LayoutPlan,
    symbols: &SymbolTable,
    opts: &SymOptions,
) -> Result<String, SymError> {
    write_sym_with_escape(layout, symbols, opts, dot_safe_name)
}

fn write_sym_with_escape(
    layout: &LayoutPlan,
    symbols: &SymbolTable,
    opts: &SymOptions,
    escape: fn(&SymbolName) -> String,
) -> Result<String, SymError> {
    let mut rewritten_names: BTreeMap<String, SymbolName> = BTreeMap::new();
    let mut entries = Vec::new();

    for (name, address) in symbols.iter() {
        let placed = layout
            .sections
            .iter()
            .find(|section| section.id == address.section)
            .ok_or(SymError::MissingPlacement {
                section_id: address.section,
            })?;
        let cpu_addr = u32::from(placed.cpu_start) + address.offset;
        let cpu_addr = u16::try_from(cpu_addr).map_err(|_| SymError::AddressOverflow {
            section_id: address.section,
            offset: address.offset,
        })?;
        let bank = match (placed.space, placed.bank) {
            (AddressSpace::Rom0, BankIndex::Rom(0)) => Some(0),
            (AddressSpace::RomX, BankIndex::Rom(bank)) => Some(bank),
            _ if opts.rom_only => continue,
            _ => None,
        };
        let rendered_name = if opts.dot_safe_separator {
            let rewritten = escape(name);
            if let Some(existing) = rewritten_names.insert(rewritten.clone(), name.clone()) {
                return Err(SymError::DotSafeNameCollision {
                    rewritten,
                    originals: [existing, name.clone()],
                });
            }
            rewritten
        } else {
            name.as_str().to_owned()
        };
        entries.push(SymSortEntry {
            location_kind: if bank.is_some() { 0 } else { 1 },
            bank,
            addr: cpu_addr,
            name: rendered_name,
        });
    }

    entries.sort();
    let mut out = String::new();
    if opts.include_externals_as_comments {
        out.push_str("; externals: none\n");
    }
    for entry in entries {
        let line = SymEntry {
            bank: entry.bank,
            addr: entry.addr,
            name: entry.name,
        };
        out.push_str(&line.to_string());
        out.push('\n');
    }
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SymSortEntry {
    location_kind: u8,
    bank: Option<u16>,
    addr: u16,
    name: String,
}

fn dot_safe_name(name: &SymbolName) -> String {
    let mut out = String::from("gbf_");
    for ch in name.as_str().chars() {
        match ch {
            '.' => out.push_str("_d"),
            '_' => out.push_str("__"),
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
#[test]
fn canonical_naming() {
    let kernel = SymbolName::kernel("matvec", 7).expect("kernel symbol");
    let expert = SymbolName::expert(2, 7).expect("expert symbol");
    let runtime = SymbolName::runtime("banking", "lease_enter").expect("runtime symbol");
    let section =
        SymbolName::section(SectionRole::Bank0Nucleus, SectionId::new(1)).expect("section symbol");

    assert_eq!(kernel.as_str(), "kernel.matvec.7");
    assert_eq!(expert.as_str(), "expert.2.7");
    assert_eq!(runtime.as_str(), "runtime.banking.lease_enter");
    assert_eq!(section.as_str(), "section.bank0_nucleus.1");
    assert_ne!(kernel, expert);
    assert_ne!(kernel, runtime);
    assert_ne!(expert, runtime);

    assert!(SymbolName::new("").is_err());
    assert!(SymbolName::new("runtime..panic").is_err());
    assert!(SymbolName::new("Runtime.Banking").is_err());
    assert!(SymbolSegment::new("banking.lease").is_err());
    assert!(SymbolName::kernel("mat.vec", 0).is_err());
    assert!(SymbolName::runtime("banking.lease", "enter").is_err());
    assert!(SymbolName::runtime("banking", "lease.enter").is_err());
    assert!(serde_json::from_str::<SymbolName>(r#""runtime..panic""#).is_err());
    assert_eq!(
        SymbolName::runtime_thunk_for(&kernel)
            .expect("thunk name")
            .as_str(),
        "runtime.banking.thunk.kernel.matvec.7"
    );

    let mut table = SymbolTable::new();
    let kernel_addr = SymbolAddress::new(SectionId::new(1), 0x20);
    let runtime_addr = SymbolAddress::new(SectionId::new(2), 0x08);
    table
        .insert(kernel.clone(), kernel_addr)
        .expect("insert kernel");
    table
        .insert(runtime.clone(), runtime_addr)
        .expect("insert runtime");

    assert_eq!(table.resolve(&kernel), Some(kernel_addr));
    assert_eq!(table.names_for(runtime_addr), vec![&runtime]);
    assert_eq!(table.len(), 2);

    assert_eq!(
        table.insert(kernel.clone(), SymbolAddress::new(SectionId::new(3), 0)),
        Err(SymbolTableError::DuplicateName {
            name: kernel,
            existing: kernel_addr,
            new: SymbolAddress::new(SectionId::new(3), 0),
        })
    );
    table
        .insert(expert.clone(), runtime_addr)
        .expect("aliases at same address are valid");
    assert_eq!(table.names_for(runtime_addr), vec![&expert, &runtime]);

    let encoded = serde_json::to_string(&table).expect("symbol table serializes to json");
    let decoded: SymbolTable = serde_json::from_str(&encoded).expect("symbol table deserializes");
    assert_eq!(decoded, table);
}

#[cfg(test)]
mod sym_tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::layout::{AddressSpace, BankIndex, LayoutPlan, PlacedSection};

    fn sym(value: &'static str) -> SymbolName {
        SymbolName::new(value).expect("symbol")
    }

    fn layout() -> LayoutPlan {
        LayoutPlan {
            sections: vec![
                PlacedSection {
                    id: SectionId::new(1),
                    space: AddressSpace::Rom0,
                    bank: BankIndex::Rom(0),
                    cpu_start: 0x0150,
                    final_size: 4,
                    estimated_size: 4,
                    alignment_padding: BTreeMap::new(),
                },
                PlacedSection {
                    id: SectionId::new(2),
                    space: AddressSpace::RomX,
                    bank: BankIndex::Rom(0x100),
                    cpu_start: 0x4000,
                    final_size: 4,
                    estimated_size: 4,
                    alignment_padding: BTreeMap::new(),
                },
                PlacedSection {
                    id: SectionId::new(3),
                    space: AddressSpace::Wram,
                    bank: BankIndex::Wram,
                    cpu_start: 0xC000,
                    final_size: 4,
                    estimated_size: 4,
                    alignment_padding: BTreeMap::new(),
                },
            ],
            bank_count: 0x101,
            free_bytes_per_bank: BTreeMap::new(),
            reserved_ranges: Vec::new(),
        }
    }

    fn table() -> SymbolTable {
        let mut symbols = SymbolTable::new();
        symbols
            .insert(
                sym("runtime.tiny.loop"),
                SymbolAddress::new(SectionId::new(1), 2),
            )
            .expect("insert");
        symbols
            .insert(sym("expert.1.0"), SymbolAddress::new(SectionId::new(2), 0))
            .expect("insert");
        symbols
            .insert(
                sym("runtime.tiny.entry"),
                SymbolAddress::new(SectionId::new(1), 0),
            )
            .expect("insert");
        symbols
            .insert(
                sym("runtime.tiny.wram"),
                SymbolAddress::new(SectionId::new(3), 0),
            )
            .expect("insert");
        symbols
    }

    #[test]
    fn write_sym_sorted() {
        let out = write_sym(
            &layout(),
            &table(),
            &SymOptions {
                include_externals_as_comments: false,
                ..SymOptions::default()
            },
        )
        .expect("write sym");
        assert_eq!(
            out,
            "00:0150 runtime.tiny.entry\n00:0152 runtime.tiny.loop\n100:4000 expert.1.0\nC000 runtime.tiny.wram\n"
        );
        let parsed = parse_sym_entries(&out).expect("parse sym");
        assert_eq!(parsed[0].bank, Some(0));
        assert_eq!(parsed[2].bank, Some(0x100));
        assert_eq!(parsed[3].bank, None);
    }

    #[test]
    fn write_sym_dot_safe() {
        let out = write_sym(
            &layout(),
            &table(),
            &SymOptions {
                include_externals_as_comments: false,
                dot_safe_separator: true,
                ..SymOptions::default()
            },
        )
        .expect("write sym");
        for line in out.lines() {
            let (_, name) = line.split_once(' ').expect("name");
            assert!(name.starts_with("gbf_"));
            assert!(!name.contains('.'));
        }
        assert!(out.contains("gbf_runtime_dtiny_dentry"));
    }

    #[test]
    fn write_sym_dot_safe_escape_avoids_naive_collision() {
        let mut symbols = SymbolTable::new();
        symbols
            .insert(sym("foo.bar_baz"), SymbolAddress::new(SectionId::new(1), 0))
            .expect("insert");
        symbols
            .insert(sym("foo_bar.baz"), SymbolAddress::new(SectionId::new(1), 1))
            .expect("insert");
        let out = write_sym(
            &layout(),
            &symbols,
            &SymOptions {
                include_externals_as_comments: false,
                dot_safe_separator: true,
                ..SymOptions::default()
            },
        )
        .expect("write sym");
        assert!(out.contains("gbf_foo_dbar__baz"));
        assert!(out.contains("gbf_foo__bar_dbaz"));
    }

    #[test]
    fn write_sym_dot_safe_collision_detected() {
        fn lossy(_: &SymbolName) -> String {
            "same".to_owned()
        }
        let mut symbols = SymbolTable::new();
        symbols
            .insert(sym("a.b"), SymbolAddress::new(SectionId::new(1), 0))
            .expect("insert");
        symbols
            .insert(sym("a_b"), SymbolAddress::new(SectionId::new(1), 1))
            .expect("insert");
        let err = write_sym_with_escape(
            &layout(),
            &symbols,
            &SymOptions {
                dot_safe_separator: true,
                ..SymOptions::default()
            },
            lossy,
        )
        .expect_err("collision");
        assert!(matches!(err, SymError::DotSafeNameCollision { .. }));
    }

    #[test]
    fn write_sym_dot_safe_collision_table_driven() {
        let names = [
            "a.b_c", "a_b.c", "x.y.z", "x_y.z", "x.y_z", "foo__bar", "foo.bar",
        ];
        let mut rewritten = std::collections::BTreeSet::new();
        for name in names {
            assert!(rewritten.insert(dot_safe_name(&sym(name))), "{name}");
        }
    }

    #[test]
    fn rom_only_omits_bankless_symbols() {
        let out = write_sym(
            &layout(),
            &table(),
            &SymOptions {
                include_externals_as_comments: false,
                rom_only: true,
                ..SymOptions::default()
            },
        )
        .expect("write sym");
        assert!(!out.contains("C000"));
    }
}
