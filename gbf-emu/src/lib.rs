//! Single-substrate Game Boy emulator adapter for tests, benches, and debugging.

#![forbid(unsafe_code)]

pub mod adapter;
pub mod determinism;
pub mod harness;
pub mod primitives;
pub mod trace_ring;
pub mod trap;

pub use adapter::{BootMode, BootRomImage, Emulator, EmulatorBuilder, EmulatorConfig};
pub use determinism::{
    AudioOutputMode, CartridgeRtcMode, DeterminismPolicy, DeterminismPolicyBuilder,
    FIXED_CARTRIDGE_RTC_UNIX_MS, FIXED_SAVE_STATE_UNIX_MS, PowerOnRamPolicy, SaveStateMetadataMode,
};
pub use harness::{HarnessChannel, HarnessCommand, HarnessResult, HarnessSlot};
pub use primitives::{
    BootModeLineage, ClockCycles, Color, CpuIdleState, CycleBudget, DMG_FRAME_CLOCK_CYCLES,
    EmuError, EmuVersionTag, Flags, Framebuffer, GitSha, ImeSnapshot, JoypadFrame, MCycles, Regs,
    RunOutcome, Snapshot, SnapshotLineage, StepOutcome, TrapPredicateError,
};
pub use trace_ring::{
    BankSnapshot, BankSwitchSource, NormalizedTraceEvent, TraceCursor, TraceDropPolicy,
    TraceMapper, TraceOrigin,
};
pub use trap::{
    AddressRange, AddressRangeError, BreakpointId, EmuReadOnlyMemory, EmuReadOnlyView,
    MemoryAccess, MemoryAccessKind, Predicate, PredicateSpec, RemovedTrap, TrapAction, TrapContext,
    TrapDispatcher, TrapKind, TrapListEntry, TrapPersistenceError, TrapSpec,
};
