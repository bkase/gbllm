//! Agent-facing scripted debugger CLI for Game Boy ROMs.
//!
//! See `history/rfcs/F-A8-gbf-debug.md` for the source-of-truth design.

#![forbid(unsafe_code)]

pub mod cli;
pub mod script;
pub mod session;

pub use cli::{
    CliError, ErrorEnvelope, ExecArgs, ExecEnvelope, ExecMetrics, InitArgs, InitEnvelope,
    InspectArgs, InspectEnvelope, RegsSnapshot, SymbolsSummary, TraceRingSummary, run_exec,
    run_init, run_inspect,
};
pub use script::{LogEntry, ScriptConfig, ScriptError, ScriptHost, ScriptOutcome, Warning};
pub use session::{
    BreakpointPersisted, EmulatorSnapshotBlob, PersistedPredicate, RomBlob, SCHEMA_VERSION,
    Session, SessionLoadError, SessionMetadata, SessionSymbolEntry, SessionSymbolTable,
    SessionWriteError, SymbolHydration, SymbolHydrationError, SymbolResolutionError,
    TraceEventKind, TraceEventPersisted, TraceRing, WatchpointKind, WatchpointPersisted,
};

pub type InitOutcome = InitEnvelope;
pub type ExecOutcome = ExecEnvelope;
pub type InspectOutcome = InspectEnvelope;
