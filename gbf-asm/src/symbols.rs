//! Canonical symbol names and post-layout symbol resolution.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

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

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
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
