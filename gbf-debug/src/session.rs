#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use gbf_asm::symbols::parse_sym_entries;
use gbf_emu::{BootModeLineage, NormalizedTraceEvent, Snapshot};
use gbf_foundation::Hash256;
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::script::Warning;

pub const SCHEMA_VERSION: u32 = 1;
const MAGIC: [u8; 4] = *b"GBSE";
const HEADER_LEN: usize = 8;
const FLAGS: u32 = 0;
const ZSTD_LEVEL: i32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: u32,
    pub parent_sha256: Option<[u8; 32]>,
    pub rom_sha256: [u8; 32],
    pub rom: RomBlob,
    pub emulator_snapshot: EmulatorSnapshotBlob,
    pub symbols: SessionSymbolTable,
    pub breakpoints: Vec<BreakpointPersisted>,
    pub watchpoints: Vec<WatchpointPersisted>,
    pub trace_ring: TraceRing,
    pub metadata: SessionMetadata,
}

impl Session {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SessionLoadError> {
        let bytes = fs::read(path).map_err(SessionLoadError::Io)?;
        Self::load_bytes(&bytes)
    }

    pub fn load_bytes(bytes: &[u8]) -> Result<Self, SessionLoadError> {
        if bytes.len() < HEADER_LEN {
            return Err(SessionLoadError::Truncated {
                observed: bytes.len(),
                minimum: HEADER_LEN,
            });
        }
        let mut observed_magic = [0_u8; 4];
        observed_magic.copy_from_slice(&bytes[..4]);
        if observed_magic != MAGIC {
            return Err(SessionLoadError::BadMagic {
                observed: observed_magic,
                expected: MAGIC,
            });
        }
        let flags = u32::from_le_bytes(bytes[4..8].try_into().expect("slice len checked"));
        if flags != FLAGS {
            return Err(SessionLoadError::BadFlags { observed: flags });
        }
        let json = zstd::decode_all(&bytes[HEADER_LEN..])
            .map_err(|error| SessionLoadError::ZstdDecode(error.to_string()))?;
        let session: Self = serde_json::from_slice(&json)
            .map_err(|error| SessionLoadError::JsonDecode(error.to_string()))?;
        session.validate()?;
        Ok(session)
    }

    pub fn validate(&self) -> Result<(), SessionLoadError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(SessionLoadError::SchemaMismatch {
                observed: self.schema_version,
                current: SCHEMA_VERSION,
            });
        }
        let observed = sha256_bytes(&self.rom.0);
        if observed != self.rom_sha256 {
            return Err(SessionLoadError::RomHashMismatch {
                observed,
                expected: self.rom_sha256,
            });
        }
        let snapshot_hash = self.emulator_snapshot.0.lineage.rom_sha256.to_bytes();
        if snapshot_hash != self.rom_sha256 {
            return Err(SessionLoadError::SnapshotRomMismatch {
                snapshot_rom_sha256: snapshot_hash,
                session_rom_sha256: self.rom_sha256,
            });
        }
        if self.emulator_snapshot.0.lineage.boot != BootModeLineage::PostBootDmg {
            return Err(SessionLoadError::UnsupportedBootMode {
                observed: format!("{:?}", self.emulator_snapshot.0.lineage.boot),
            });
        }
        Ok(())
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>, SessionWriteError> {
        let json = serde_json::to_vec(self)
            .map_err(|error| SessionWriteError::JsonEncode(error.to_string()))?;
        let compressed = zstd::encode_all(&json[..], ZSTD_LEVEL)
            .map_err(|error| SessionWriteError::ZstdEncode(error.to_string()))?;
        let mut out = Vec::with_capacity(HEADER_LEN + compressed.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&FLAGS.to_le_bytes());
        out.extend_from_slice(&compressed);
        Ok(out)
    }

    pub fn sha256(&self) -> Result<[u8; 32], SessionWriteError> {
        Ok(sha256_bytes(&self.to_bytes()?))
    }

    pub fn write(&self, path: impl AsRef<Path>) -> Result<[u8; 32], SessionWriteError> {
        self.replace(path)
    }

    pub fn write_new(&self, path: impl AsRef<Path>) -> Result<[u8; 32], SessionWriteError> {
        write_atomic(self, path.as_ref(), false)
    }

    pub fn replace(&self, path: impl AsRef<Path>) -> Result<[u8; 32], SessionWriteError> {
        write_atomic(self, path.as_ref(), true)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub abi_version_observed: Option<gbf_abi::AbiVersion>,
    pub created_at_micros_since_init: u64,
    pub notes: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomBlob(pub Vec<u8>);

impl Serialize for RomBlob {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&BASE64.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for RomBlob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        BASE64
            .decode(encoded)
            .map(Self)
            .map_err(|error| D::Error::custom(error.to_string()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmulatorSnapshotBlob(pub Snapshot);

#[derive(Serialize, Deserialize)]
struct EmulatorSnapshotRepr {
    blob: String,
    lineage: gbf_emu::SnapshotLineage,
    trace_bank: gbf_emu::BankSnapshot,
}

impl Serialize for EmulatorSnapshotBlob {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        EmulatorSnapshotRepr {
            blob: BASE64.encode(&self.0.blob),
            lineage: self.0.lineage,
            trace_bank: self.0.trace_bank,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for EmulatorSnapshotBlob {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let repr = EmulatorSnapshotRepr::deserialize(deserializer)?;
        let blob = BASE64
            .decode(repr.blob)
            .map_err(|error| D::Error::custom(error.to_string()))?;
        Ok(Self(Snapshot {
            blob,
            lineage: repr.lineage,
            trace_bank: repr.trace_bank,
        }))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSymbolTable {
    pub entries: Vec<SessionSymbolEntry>,
}

impl SessionSymbolTable {
    pub fn from_sym_text(input: &str) -> Result<SymbolHydration, SymbolHydrationError> {
        let mut entries: Vec<_> = parse_sym_entries(input)
            .map_err(|error| SymbolHydrationError::SymParse(error.to_string()))?
            .into_iter()
            .map(|entry| SessionSymbolEntry {
                bank: entry.bank,
                addr: entry.addr,
                name: entry.name,
            })
            .collect();
        entries.sort_by(|a, b| (a.bank, a.addr, &a.name).cmp(&(b.bank, b.addr, &b.name)));

        let mut seen = BTreeMap::<String, u32>::new();
        for entry in &entries {
            *seen.entry(entry.name.clone()).or_default() += 1;
        }
        let warnings = seen
            .into_iter()
            .filter_map(|(name, count)| {
                (count > 1).then(|| {
                    Warning::new(
                        "duplicate_symbol_name",
                        serde_json::json!({
                            "name": name,
                            "count": count,
                        }),
                    )
                })
            })
            .collect();

        Ok(SymbolHydration {
            table: Self { entries },
            warnings,
        })
    }

    pub fn resolve(&self, name: &str) -> Result<Option<u16>, SymbolResolutionError> {
        let candidates: Vec<_> = self
            .entries
            .iter()
            .filter(|entry| entry.name == name)
            .cloned()
            .collect();
        match candidates.as_slice() {
            [] => Ok(None),
            [entry] => Ok(Some(entry.addr)),
            _ => Err(SymbolResolutionError::AmbiguousName {
                name: name.to_owned(),
                candidates,
            }),
        }
    }

    #[must_use]
    pub fn resolve_in_bank(&self, name: &str, bank: u16) -> Option<u16> {
        self.entries
            .iter()
            .find(|entry| entry.name == name && entry.bank == Some(bank))
            .map(|entry| entry.addr)
    }

    pub fn resolve_at(&self, addr: u16) -> Result<Option<&str>, SymbolResolutionError> {
        let mut matches = self.entries.iter().filter(|entry| entry.addr == addr);
        let Some(first) = matches.next() else {
            return Ok(None);
        };
        if matches.next().is_none() {
            return Ok(Some(first.name.as_str()));
        }
        let candidates = self
            .entries
            .iter()
            .filter(|entry| entry.addr == addr)
            .cloned()
            .collect();
        Err(SymbolResolutionError::AmbiguousName {
            name: format!("${addr:04X}"),
            candidates,
        })
    }

    #[must_use]
    pub fn resolve_at_in_bank(&self, addr: u16, bank: u16) -> Option<&str> {
        self.entries
            .iter()
            .find(|entry| entry.addr == addr && entry.bank == Some(bank))
            .map(|entry| entry.name.as_str())
    }

    #[must_use]
    pub fn summary(&self) -> (u32, u32, u32) {
        let count = self.entries.len() as u32;
        let banked = self
            .entries
            .iter()
            .filter(|entry| entry.bank.is_some())
            .count() as u32;
        (count, banked, count - banked)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionSymbolEntry {
    pub bank: Option<u16>,
    pub addr: u16,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHydration {
    pub table: SessionSymbolTable,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolHydrationError {
    SymParse(String),
}

impl fmt::Display for SymbolHydrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SymParse(message) => write!(f, ".sym parse failed: {message}"),
        }
    }
}

impl std::error::Error for SymbolHydrationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolResolutionError {
    AmbiguousName {
        name: String,
        candidates: Vec<SessionSymbolEntry>,
    },
}

impl fmt::Display for SymbolResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousName { name, candidates } => {
                write!(
                    f,
                    "symbol {name:?} is ambiguous across {} entries",
                    candidates.len()
                )
            }
        }
    }
}

impl std::error::Error for SymbolResolutionError {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakpointPersisted {
    pub addr: u16,
    pub predicate: PersistedPredicate,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WatchpointPersisted {
    pub addr: u16,
    pub kind: WatchpointKind,
    pub predicate: PersistedPredicate,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum WatchpointKind {
    Read,
    Write,
    ReadWrite,
}

impl WatchpointKind {
    pub const ALL: [Self; 3] = [Self::Read, Self::Write, Self::ReadWrite];

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read" => Some(Self::Read),
            "write" => Some(Self::Write),
            "rw" | "read_write" => Some(Self::ReadWrite),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::ReadWrite => "rw",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedPredicate {
    None,
    StringifiedSource(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRing {
    pub capacity: u32,
    pub events: VecDeque<TraceEventPersisted>,
    pub dropped: u64,
    pub next_seq: u64,
}

impl TraceRing {
    #[must_use]
    pub fn new(capacity: u32) -> Self {
        Self {
            capacity,
            events: VecDeque::with_capacity(capacity as usize),
            dropped: 0,
            next_seq: 0,
        }
    }

    pub fn push(&mut self, mut event: TraceEventPersisted) {
        if self.capacity == 0 {
            self.dropped = self.dropped.saturating_add(1);
            self.next_seq = self.next_seq.saturating_add(1);
            return;
        }
        if self.events.len() == self.capacity as usize {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        event.seq = self.next_seq;
        self.next_seq = self.next_seq.saturating_add(1);
        self.events.push_back(event);
    }

    pub fn extend_normalized(&mut self, events: Vec<NormalizedTraceEvent>, pc_at: u16) {
        for event in events {
            if let Some(persisted) = TraceEventPersisted::from_normalized(event, pc_at) {
                self.push(persisted);
            }
        }
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.dropped = 0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEventPersisted {
    pub seq: u64,
    pub kind: TraceEventKind,
    pub addr: u16,
    pub data: Vec<u8>,
    pub pc_at: u16,
}

impl TraceEventPersisted {
    #[must_use]
    pub const fn step_boundary(pc_at: u16) -> Self {
        Self {
            seq: 0,
            kind: TraceEventKind::StepBoundary,
            addr: pc_at,
            data: Vec::new(),
            pc_at,
        }
    }

    #[must_use]
    pub fn from_normalized(event: NormalizedTraceEvent, pc_at: u16) -> Option<Self> {
        match event {
            NormalizedTraceEvent::MemoryWrite { addr, value, .. } => Some(Self {
                seq: 0,
                kind: TraceEventKind::MemoryWrite,
                addr,
                data: vec![value],
                pc_at,
            }),
            NormalizedTraceEvent::RomBankSwitch { to, .. } => Some(Self {
                seq: 0,
                kind: TraceEventKind::RomBankSwitch,
                addr: 0,
                data: to.to_le_bytes().to_vec(),
                pc_at,
            }),
            NormalizedTraceEvent::SramBankSwitch { to, .. } => Some(Self {
                seq: 0,
                kind: TraceEventKind::SramBankSwitch,
                addr: 0,
                data: vec![to],
                pc_at,
            }),
            NormalizedTraceEvent::IoWrite { reg, value, .. } => Some(Self {
                seq: 0,
                kind: TraceEventKind::IoWrite,
                addr: reg,
                data: vec![value],
                pc_at,
            }),
            NormalizedTraceEvent::TrapHit { trap_id, kind, .. } => Some(Self {
                seq: 0,
                kind: TraceEventKind::TrapHit,
                addr: trap_addr(kind),
                data: trap_id.0.to_le_bytes().to_vec(),
                pc_at,
            }),
            NormalizedTraceEvent::Typed(_) => Some(Self {
                seq: 0,
                kind: TraceEventKind::Typed,
                addr: 0,
                data: Vec::new(),
                pc_at,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraceEventKind {
    MemoryWrite,
    RomBankSwitch,
    SramBankSwitch,
    IoWrite,
    TrapHit,
    Typed,
    StepBoundary,
}

#[derive(Debug)]
pub enum SessionLoadError {
    Io(std::io::Error),
    BadMagic {
        observed: [u8; 4],
        expected: [u8; 4],
    },
    BadFlags {
        observed: u32,
    },
    Truncated {
        observed: usize,
        minimum: usize,
    },
    ZstdDecode(String),
    JsonDecode(String),
    SchemaMismatch {
        observed: u32,
        current: u32,
    },
    RomHashMismatch {
        observed: [u8; 32],
        expected: [u8; 32],
    },
    SnapshotRomMismatch {
        snapshot_rom_sha256: [u8; 32],
        session_rom_sha256: [u8; 32],
    },
    UnsupportedBootMode {
        observed: String,
    },
}

impl fmt::Display for SessionLoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "session I/O failed: {error}"),
            Self::BadMagic { observed, expected } => {
                write!(f, "bad magic {observed:?}; expected {expected:?}")
            }
            Self::BadFlags { observed } => write!(f, "bad flags {observed:#010x}"),
            Self::Truncated { observed, minimum } => {
                write!(
                    f,
                    "session truncated: {observed} bytes, need at least {minimum}"
                )
            }
            Self::ZstdDecode(message) => write!(f, "zstd decode failed: {message}"),
            Self::JsonDecode(message) => write!(f, "json decode failed: {message}"),
            Self::SchemaMismatch { observed, current } => {
                write!(f, "schema mismatch: observed {observed}, current {current}")
            }
            Self::RomHashMismatch { .. } => f.write_str("embedded ROM hash mismatch"),
            Self::SnapshotRomMismatch { .. } => f.write_str("snapshot ROM hash mismatch"),
            Self::UnsupportedBootMode { observed } => {
                write!(
                    f,
                    "unsupported boot mode {observed}; F-A8 sessions are PostBootDmg-only"
                )
            }
        }
    }
}

impl std::error::Error for SessionLoadError {}

#[derive(Debug)]
pub enum SessionWriteError {
    Io(std::io::Error),
    JsonEncode(String),
    ZstdEncode(String),
}

impl fmt::Display for SessionWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "session write I/O failed: {error}"),
            Self::JsonEncode(message) => write!(f, "session json encode failed: {message}"),
            Self::ZstdEncode(message) => write!(f, "session zstd encode failed: {message}"),
        }
    }
}

impl std::error::Error for SessionWriteError {}

pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn hex_hash(bytes: [u8; 32]) -> String {
    Hash256::from(bytes).to_string()
}

fn write_atomic(
    session: &Session,
    path: &Path,
    replace_existing: bool,
) -> Result<[u8; 32], SessionWriteError> {
    let bytes = session.to_bytes()?;
    let sha = sha256_bytes(&bytes);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(SessionWriteError::Io)?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".tmp-gbsess-")
        .tempfile_in(parent)
        .map_err(SessionWriteError::Io)?;
    tmp.write_all(&bytes).map_err(SessionWriteError::Io)?;
    tmp.as_file_mut()
        .sync_all()
        .map_err(SessionWriteError::Io)?;
    if replace_existing {
        tmp.persist(path)
            .map_err(|error| SessionWriteError::Io(error.error))?;
    } else {
        tmp.persist_noclobber(path)
            .map_err(|error| SessionWriteError::Io(error.error))?;
    }
    sync_parent_dir(parent)?;
    Ok(sha)
}

fn sync_parent_dir(parent: &Path) -> Result<(), SessionWriteError> {
    let file = File::open(parent).map_err(SessionWriteError::Io)?;
    file.sync_all().map_err(SessionWriteError::Io)
}

fn trap_addr(kind: gbf_emu::TrapKind) -> u16 {
    match kind {
        gbf_emu::TrapKind::Pc { addr } => addr,
        gbf_emu::TrapKind::MemRead { range }
        | gbf_emu::TrapKind::MemWrite { range }
        | gbf_emu::TrapKind::MemRw { range } => range.start(),
    }
}

pub fn unique_banks_for_name(table: &SessionSymbolTable, name: &str) -> BTreeSet<Option<u16>> {
    table
        .entries
        .iter()
        .filter(|entry| entry.name == name)
        .map(|entry| entry.bank)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gbf_emu::{
        BankSnapshot, BootModeLineage, ClockCycles, EmuVersionTag, GitSha, SnapshotLineage,
    };
    use gbf_foundation::SemVer;

    fn snapshot(hash: [u8; 32]) -> EmulatorSnapshotBlob {
        EmulatorSnapshotBlob(Snapshot {
            blob: vec![1, 2, 3],
            lineage: SnapshotLineage {
                rom_sha256: Hash256::from(hash),
                boot: BootModeLineage::PostBootDmg,
                policy_fingerprint: Hash256::ZERO,
                emu_version: EmuVersionTag {
                    gameroy_package: "gameroy-core",
                    gameroy_semver: SemVer::new(0, 1, 0),
                    gameroy_git_rev: GitSha::ZERO,
                    gbf_emu_version: SemVer::new(0, 1, 0),
                },
                cycle_count: ClockCycles(0),
            },
            trace_bank: BankSnapshot::default(),
        })
    }

    fn session() -> Session {
        let rom = RomBlob(vec![0, 1, 2, 3]);
        let rom_sha256 = sha256_bytes(&rom.0);
        Session {
            schema_version: SCHEMA_VERSION,
            parent_sha256: None,
            rom_sha256,
            rom,
            emulator_snapshot: snapshot(rom_sha256),
            symbols: SessionSymbolTable::default(),
            breakpoints: Vec::new(),
            watchpoints: Vec::new(),
            trace_ring: TraceRing::new(2),
            metadata: SessionMetadata::default(),
        }
    }

    #[test]
    fn magic_round_trip() {
        let bytes = session().to_bytes().expect("serialize");
        assert_eq!(&bytes[..4], b"GBSE");
        assert_eq!(Session::load_bytes(&bytes).expect("load"), session());
    }

    #[test]
    fn flags_must_be_zero() {
        let mut bytes = session().to_bytes().expect("serialize");
        bytes[4] = 1;
        assert!(matches!(
            Session::load_bytes(&bytes),
            Err(SessionLoadError::BadFlags { observed: 1 })
        ));
    }

    #[test]
    fn schema_mismatch_is_fatal() {
        let mut session = session();
        session.schema_version = 999;
        let bytes = session.to_bytes().expect("serialize");
        assert!(matches!(
            Session::load_bytes(&bytes),
            Err(SessionLoadError::SchemaMismatch { observed: 999, .. })
        ));
    }

    #[test]
    fn trace_ring_capped() {
        let mut ring = TraceRing::new(2);
        ring.push(TraceEventPersisted::step_boundary(1));
        ring.push(TraceEventPersisted::step_boundary(2));
        ring.push(TraceEventPersisted::step_boundary(3));
        assert_eq!(ring.events.len(), 2);
        assert_eq!(ring.dropped, 1);
        assert_eq!(ring.events.front().expect("front").pc_at, 2);
    }

    #[test]
    fn breakpoint_predicate_round_trip() {
        let encoded = serde_json::to_string(&BreakpointPersisted {
            addr: 0x0150,
            predicate: PersistedPredicate::StringifiedSource("regs.a == 0x42".to_owned()),
            enabled: true,
        })
        .expect("serialize");
        let decoded: BreakpointPersisted = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.addr, 0x0150);
    }

    #[test]
    fn watchpoint_kinds_exhaustive() {
        assert_eq!(WatchpointKind::ALL.len(), 3);
        assert_eq!(WatchpointKind::parse("rw"), Some(WatchpointKind::ReadWrite));
    }

    #[test]
    fn symbols_duplicate_name_warned_not_fatal() {
        let hydrated =
            SessionSymbolTable::from_sym_text("00:0100 same\n01:4000 same\n").expect("hydrated");
        assert_eq!(hydrated.table.entries.len(), 2);
        assert_eq!(hydrated.warnings.len(), 1);
        assert!(matches!(
            hydrated.table.resolve("same"),
            Err(SymbolResolutionError::AmbiguousName { .. })
        ));
        assert_eq!(hydrated.table.resolve_in_bank("same", 1), Some(0x4000));
    }
}
