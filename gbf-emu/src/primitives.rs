//! Public value objects shared by the emulator adapter modules.

use core::fmt;
use std::error::Error;
use std::str::FromStr;

use gbf_foundation::{Hash256, SemVer};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::trap::{BreakpointId, TrapKind};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct ClockCycles(pub u64);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct MCycles(pub u64);

/// DMG frame duration in CPU clock cycles: 154 scanlines * 456 cycles.
pub const DMG_FRAME_CLOCK_CYCLES: ClockCycles = ClockCycles(154 * 456);

impl From<MCycles> for ClockCycles {
    fn from(value: MCycles) -> Self {
        Self(value.0.saturating_mul(4))
    }
}

impl ClockCycles {
    #[must_use]
    pub const fn as_m_cycles_floor(self) -> MCycles {
        MCycles(self.0 / 4)
    }

    /// Saturating helper for frame-count budgets.
    #[must_use]
    pub const fn saturating_mul(self, rhs: u64) -> Self {
        Self(self.0.saturating_mul(rhs))
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CycleBudget {
    Clock(ClockCycles),
    Machine(MCycles),
}

impl CycleBudget {
    #[must_use]
    pub fn as_clock_cycles(self) -> ClockCycles {
        match self {
            Self::Clock(cycles) => cycles,
            Self::Machine(cycles) => cycles.into(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum CpuIdleState {
    Halt,
    Stop,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum StepOutcome {
    Stepped {
        cycles: ClockCycles,
    },
    TrapHit {
        trap_id: BreakpointId,
        kind: TrapKind,
        cycles: ClockCycles,
    },
    Idle {
        state: CpuIdleState,
        cycles: ClockCycles,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum RunOutcome {
    BudgetElapsed {
        observed: ClockCycles,
        requested: ClockCycles,
    },
    TrapHit {
        trap_id: BreakpointId,
        kind: TrapKind,
        observed: ClockCycles,
    },
    Idle {
        state: CpuIdleState,
        observed: ClockCycles,
    },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Flags(u8);

impl Flags {
    #[must_use]
    pub const fn new(masked: u8) -> Self {
        Self(masked & 0xF0)
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn z(self) -> bool {
        (self.0 & 0x80) != 0
    }

    #[must_use]
    pub const fn n(self) -> bool {
        (self.0 & 0x40) != 0
    }

    #[must_use]
    pub const fn h(self) -> bool {
        (self.0 & 0x20) != 0
    }

    #[must_use]
    pub const fn c(self) -> bool {
        (self.0 & 0x10) != 0
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ImeSnapshot {
    Disabled,
    Enabled,
    ToBeEnable,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Regs {
    pub a: u8,
    pub f: Flags,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: ImeSnapshot,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Framebuffer {
    pixels: [u8; Self::WIDTH * Self::HEIGHT],
}

impl Framebuffer {
    pub const WIDTH: usize = 160;
    pub const HEIGHT: usize = 144;

    #[must_use]
    pub const fn from_pixels(pixels: [u8; Self::WIDTH * Self::HEIGHT]) -> Self {
        Self { pixels }
    }

    #[must_use]
    pub fn pixel(&self, x: usize, y: usize) -> u8 {
        self.pixels[y * Self::WIDTH + x]
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; Self::WIDTH * Self::HEIGHT] {
        &self.pixels
    }

    #[must_use]
    pub const fn dmg_palette() -> [Color; 4] {
        [
            Color {
                r: 0xE0,
                g: 0xF8,
                b: 0xD0,
            },
            Color {
                r: 0x88,
                g: 0xC0,
                b: 0x70,
            },
            Color {
                r: 0x34,
                g: 0x68,
                b: 0x56,
            },
            Color {
                r: 0x08,
                g: 0x18,
                b: 0x20,
            },
        ]
    }
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct JoypadFrame {
    bits: u8,
}

impl JoypadFrame {
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self { bits }
    }

    #[must_use]
    pub fn pressed(button: gbf_hw::joypad::Button) -> Self {
        Self::default().with(button)
    }

    #[must_use]
    pub fn with(mut self, button: gbf_hw::joypad::Button) -> Self {
        self.bits |= button.state_mask();
        self
    }

    #[must_use]
    pub fn is_pressed(&self, button: gbf_hw::joypad::Button) -> bool {
        (self.bits & button.state_mask()) != 0
    }

    #[must_use]
    pub const fn bits(self) -> u8 {
        self.bits
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub blob: Vec<u8>,
    pub lineage: SnapshotLineage,
    pub trace_bank: crate::trace_ring::BankSnapshot,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum BootModeLineage {
    PostBootDmg,
    BootRom { sha256: Hash256 },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SnapshotLineage {
    pub rom_sha256: Hash256,
    pub boot: BootModeLineage,
    pub policy_fingerprint: Hash256,
    pub emu_version: EmuVersionTag,
    pub cycle_count: ClockCycles,
}

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
pub struct GitSha([u8; 20]);

impl GitSha {
    pub const ZERO: Self = Self([0; 20]);

    #[must_use]
    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    #[must_use]
    pub const fn to_bytes(self) -> [u8; 20] {
        self.0
    }
}

impl fmt::Debug for GitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl fmt::Display for GitSha {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for GitSha {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for GitSha {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for GitSha {
    type Err = GitShaParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let hex = value.strip_prefix("0x").unwrap_or(value);
        if hex.len() != 40 {
            return Err(GitShaParseError::InvalidLength {
                expected: 40,
                actual: hex.len(),
            });
        }

        let mut bytes = [0_u8; 20];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_value(pair[0]).ok_or(GitShaParseError::InvalidHex {
                index: index * 2,
                byte: pair[0],
            })?;
            let low = hex_value(pair[1]).ok_or(GitShaParseError::InvalidHex {
                index: index * 2 + 1,
                byte: pair[1],
            })?;
            bytes[index] = (high << 4) | low;
        }
        Ok(Self(bytes))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GitShaParseError {
    InvalidLength { expected: usize, actual: usize },
    InvalidHex { index: usize, byte: u8 },
}

impl fmt::Display for GitShaParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(f, "expected {expected} hex characters, got {actual}")
            }
            Self::InvalidHex { index, byte } => {
                write!(f, "invalid hex byte 0x{byte:02x} at index {index}")
            }
        }
    }
}

impl Error for GitShaParseError {}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct EmuVersionTag {
    pub gameroy_package: &'static str,
    pub gameroy_semver: SemVer,
    pub gameroy_git_rev: GitSha,
    pub gbf_emu_version: SemVer,
}

impl EmuVersionTag {
    #[must_use]
    pub fn current() -> Self {
        Self {
            gameroy_package: "gameroy-core",
            gameroy_semver: parse_semver(env!("GBF_EMU_GAMEROY_VERSION")),
            gameroy_git_rev: env!("GBF_EMU_GAMEROY_GIT_REV")
                .parse()
                .expect("build.rs injects a full gameroy git sha"),
            gbf_emu_version: parse_semver(env!("CARGO_PKG_VERSION")),
        }
    }
}

impl Serialize for EmuVersionTag {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("EmuVersionTag", 4)?;
        state.serialize_field("gameroy_package", self.gameroy_package)?;
        state.serialize_field("gameroy_semver", &self.gameroy_semver)?;
        state.serialize_field("gameroy_git_rev", &self.gameroy_git_rev)?;
        state.serialize_field("gbf_emu_version", &self.gbf_emu_version)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for EmuVersionTag {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Repr {
            gameroy_package: String,
            gameroy_semver: SemVer,
            gameroy_git_rev: GitSha,
            gbf_emu_version: SemVer,
        }

        let repr = Repr::deserialize(deserializer)?;
        if repr.gameroy_package != "gameroy-core" {
            return Err(serde::de::Error::custom(format!(
                "unsupported emulator package {}",
                repr.gameroy_package
            )));
        }
        Ok(Self {
            gameroy_package: "gameroy-core",
            gameroy_semver: repr.gameroy_semver,
            gameroy_git_rev: repr.gameroy_git_rev,
            gbf_emu_version: repr.gbf_emu_version,
        })
    }
}

fn parse_semver(value: &str) -> SemVer {
    value
        .parse()
        .unwrap_or_else(|_| panic!("invalid semantic version injected at build time: {value}"))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrapPredicateError {
    SourceRequiresEvaluator,
    PredicateFailed { reason: String },
}

impl fmt::Display for TrapPredicateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceRequiresEvaluator => {
                f.write_str("source predicate requires an external evaluator")
            }
            Self::PredicateFailed { reason } => write!(f, "predicate failed: {reason}"),
        }
    }
}

impl Error for TrapPredicateError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmuError {
    RomLoad {
        reason: String,
    },
    Step {
        reason: String,
    },
    TrapPredicate(TrapPredicateError),
    SnapshotSave {
        reason: String,
    },
    SnapshotLoad {
        reason: String,
    },
    SnapshotRomMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    SnapshotBootMismatch {
        expected: BootModeLineage,
        observed: BootModeLineage,
    },
    SnapshotPolicyMismatch {
        expected: Hash256,
        observed: Hash256,
    },
    SnapshotEmuVersionMismatch {
        expected: Box<EmuVersionTag>,
        observed: Box<EmuVersionTag>,
    },
    HarnessMagicMismatch {
        observed: [u8; 4],
        expected: [u8; 4],
    },
    HarnessSequenceMismatch {
        observed: u32,
        expected: u32,
    },
    HarnessSramOutOfRange {
        bank: u8,
        addr: u16,
        len: usize,
        ram_len: usize,
    },
    HarnessSramAccessUnavailable {
        reason: String,
    },
    TraceCapacityExceeded {
        capacity: usize,
    },
    FastRunBlockedByMemoryTraps {
        memory_trap_count: usize,
    },
    MemoryAccess {
        addr: u16,
        reason: String,
    },
    DebugMemoryUnsupported {
        addr: u16,
    },
    Determinism {
        reason: String,
    },
}

impl fmt::Display for EmuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RomLoad { reason } => write!(f, "ROM load failed: {reason}"),
            Self::Step { reason } => write!(f, "emulator step failed: {reason}"),
            Self::TrapPredicate(error) => write!(f, "trap predicate failed: {error}"),
            Self::SnapshotSave { reason } => write!(f, "snapshot save failed: {reason}"),
            Self::SnapshotLoad { reason } => write!(f, "snapshot load failed: {reason}"),
            Self::SnapshotRomMismatch { expected, observed } => {
                write!(
                    f,
                    "snapshot ROM mismatch: expected {expected}, observed {observed}"
                )
            }
            Self::SnapshotBootMismatch { expected, observed } => {
                write!(
                    f,
                    "snapshot boot mode mismatch: expected {expected:?}, observed {observed:?}"
                )
            }
            Self::SnapshotPolicyMismatch { expected, observed } => write!(
                f,
                "snapshot determinism policy mismatch: expected {expected}, observed {observed}"
            ),
            Self::SnapshotEmuVersionMismatch { expected, observed } => write!(
                f,
                "snapshot emulator version mismatch: expected {expected:?}, observed {observed:?}"
            ),
            Self::HarnessMagicMismatch { observed, expected } => {
                write!(
                    f,
                    "harness magic mismatch: observed {observed:?}, expected {expected:?}"
                )
            }
            Self::HarnessSequenceMismatch { observed, expected } => {
                write!(
                    f,
                    "harness sequence mismatch: observed {observed}, expected {expected}"
                )
            }
            Self::HarnessSramOutOfRange {
                bank,
                addr,
                len,
                ram_len,
            } => write!(
                f,
                "harness SRAM access bank {bank}, addr {addr:#06x}, len {len} exceeds cartridge RAM len {ram_len}"
            ),
            Self::HarnessSramAccessUnavailable { reason } => {
                write!(f, "harness SRAM access unavailable: {reason}")
            }
            Self::TraceCapacityExceeded { capacity } => {
                write!(f, "trace capacity {capacity} exceeded")
            }
            Self::FastRunBlockedByMemoryTraps { memory_trap_count } => write!(
                f,
                "fast run blocked by {memory_trap_count} installed memory trap(s); remove the memory traps or call the fully instrumented runner"
            ),
            Self::MemoryAccess { addr, reason } => {
                write!(f, "memory access at {addr:#06x} failed: {reason}")
            }
            Self::DebugMemoryUnsupported { addr } => {
                write!(f, "debug memory access is unsupported at {addr:#06x}")
            }
            Self::Determinism { reason } => write!(f, "determinism policy rejected ROM: {reason}"),
        }
    }
}

impl Error for EmuError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_low_nibble_masked() {
        assert_eq!(Flags::new(0xBF).bits(), 0xB0);
    }

    #[test]
    fn cycle_budget_clock_machine_equivalence() {
        assert_eq!(
            CycleBudget::Machine(MCycles(7)).as_clock_cycles(),
            ClockCycles(28)
        );
    }

    #[test]
    fn regs_serde_round_trip() {
        let regs = Regs {
            a: 1,
            f: Flags::new(0xF5),
            b: 2,
            c: 3,
            d: 4,
            e: 5,
            h: 6,
            l: 7,
            sp: 0xFFFE,
            pc: 0x0150,
            ime: ImeSnapshot::ToBeEnable,
        };

        let encoded = serde_json::to_string(&regs).expect("regs serialize");
        let decoded: Regs = serde_json::from_str(&encoded).expect("regs deserialize");

        assert_eq!(decoded, regs);
        assert_eq!(decoded.f.bits() & 0x0F, 0);
    }

    #[test]
    fn emu_version_tag_records_git_rev() {
        assert_eq!(
            EmuVersionTag::current().gameroy_git_rev.to_string(),
            "a5acdc921c0561ed93a077622b598df0e068583c"
        );
    }
}
