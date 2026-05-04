#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use gbf_emu::{BootMode, DeterminismPolicy, Emulator, EmulatorConfig, ImeSnapshot};
use serde::Serialize;
use serde_json::Value as JsonValue;

use crate::script::{
    LogEntry, ScriptConfig, ScriptError, ScriptFailure, ScriptHost, ScriptSession,
    ScriptSessionInput, Warning,
};
use crate::session::{
    EmulatorSnapshotBlob, RomBlob, SCHEMA_VERSION, Session, SessionLoadError, SessionMetadata,
    SessionSymbolTable, SessionWriteError, TraceRing, hex_hash, sha256_bytes,
};

#[derive(Debug, Clone)]
pub struct InitArgs {
    pub rom_path: PathBuf,
    pub sym_path: Option<PathBuf>,
    pub out_path: PathBuf,
    pub trace_capacity: u32,
    pub replace_existing_out: bool,
}

#[derive(Debug, Clone)]
pub struct ExecArgs {
    pub in_path: PathBuf,
    pub script_path: PathBuf,
    pub out_path: PathBuf,
    pub config: ScriptConfig,
    pub emit_metrics: bool,
    pub write_partial_on_timeout: bool,
    pub replace_existing_out: bool,
}

#[derive(Debug, Clone)]
pub struct InspectArgs {
    pub in_path: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct InitEnvelope {
    pub command: &'static str,
    pub session_path: String,
    pub session_sha256: String,
    pub rom_sha256: String,
    pub symbol_count: u32,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecEnvelope {
    pub command: &'static str,
    pub result: JsonValue,
    pub logs: Vec<LogEntry>,
    pub session_path: String,
    pub session_sha256: String,
    pub parent_sha256: Option<String>,
    pub warnings: Vec<Warning>,
    pub metrics: Option<ExecMetrics>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecMetrics {
    pub script_micros: u64,
    pub host_setup_micros: u64,
    pub session_write_micros: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct InspectEnvelope {
    pub command: &'static str,
    pub session_path: String,
    pub session_sha256: String,
    pub schema_version: u32,
    pub parent_sha256: Option<String>,
    pub rom_sha256: String,
    pub regs: RegsSnapshot,
    pub breakpoints: Vec<crate::session::BreakpointPersisted>,
    pub watchpoints: Vec<crate::session::WatchpointPersisted>,
    pub trace_ring_summary: TraceRingSummary,
    pub symbols_summary: SymbolsSummary,
    pub metadata: SessionMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegsSnapshot {
    pub pc: u16,
    pub sp: u16,
    pub a: u8,
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub f: u8,
    pub ime: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct TraceRingSummary {
    pub capacity: u32,
    pub event_count: u32,
    pub dropped: u64,
    pub head_seq: Option<u64>,
    pub tail_seq: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolsSummary {
    pub count: u32,
    pub banked_count: u32,
    pub unbanked_count: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub command: String,
    pub kind: String,
    pub message: String,
    pub script_line: Option<u32>,
    pub script_column: Option<u32>,
    pub script_function: Option<String>,
    pub session_path: Option<String>,
    pub partial_session_path: Option<String>,
    pub partial_session_sha256: Option<String>,
    pub determinism: Option<String>,
}

impl ErrorEnvelope {
    pub fn cli_args(message: String) -> Self {
        Self {
            command: "args".to_owned(),
            kind: "cli_args".to_owned(),
            message,
            script_line: None,
            script_column: None,
            script_function: None,
            session_path: None,
            partial_session_path: None,
            partial_session_sha256: None,
            determinism: None,
        }
    }

    pub fn help(message: String) -> Self {
        Self {
            command: "args".to_owned(),
            kind: "help".to_owned(),
            message,
            script_line: None,
            script_column: None,
            script_function: None,
            session_path: None,
            partial_session_path: None,
            partial_session_sha256: None,
            determinism: None,
        }
    }

    pub fn from_cli_error(command: &str, error: &CliError) -> Self {
        let (kind, script_line, script_column, script_function) = match error {
            CliError::SessionLoad(_) => ("session_load", None, None, None),
            CliError::SessionWrite(_) => ("session_write", None, None, None),
            CliError::SymbolHydration(_) => ("symbol_hydration", None, None, None),
            CliError::ScriptError { error, .. } => match error.as_ref() {
                ScriptError::SyntaxError { line, column, .. } => {
                    ("script_syntax", Some(*line), Some(*column), None)
                }
                ScriptError::RuntimeException {
                    line,
                    column,
                    function,
                    ..
                } => ("script_runtime", *line, *column, function.clone()),
                ScriptError::Timeout { .. } => ("watchdog_timeout", None, None, None),
                ScriptError::OutOfMemory => ("script_out_of_memory", None, None, None),
                ScriptError::StackOverflow => ("script_stack_overflow", None, None, None),
                ScriptError::HostBindingError { .. } => ("host_binding", None, None, None),
            },
            CliError::PredicateCompileFailed { .. } => ("predicate_compile", None, None, None),
            CliError::PostLoadPcUnexpected { .. } => ("post_load_pc", None, None, None),
            CliError::CliArgs { .. } | CliError::InOutSamePath => ("cli_args", None, None, None),
            CliError::Io(_) | CliError::Emulator(_) => ("io", None, None, None),
        };
        let (session_path, partial_session_path, partial_session_sha256, determinism) = match error
        {
            CliError::ScriptError {
                session_path,
                partial_session_path,
                partial_session_sha256,
                determinism,
                ..
            } => (
                session_path.clone(),
                partial_session_path.clone(),
                partial_session_sha256.clone(),
                determinism.clone(),
            ),
            _ => (None, None, None, None),
        };
        Self {
            command: command.to_owned(),
            kind: kind.to_owned(),
            message: error.to_string(),
            script_line,
            script_column,
            script_function,
            session_path,
            partial_session_path,
            partial_session_sha256,
            determinism,
        }
    }
}

pub fn run_init(args: InitArgs) -> Result<InitEnvelope, CliError> {
    let rom = fs::read(&args.rom_path).map_err(CliError::Io)?;
    let rom_sha256 = sha256_bytes(&rom);
    let emulator = load_emulator(&rom)?;
    let regs = emulator.regs();
    if regs.pc != 0x0100 {
        return Err(CliError::PostLoadPcUnexpected { observed: regs.pc });
    }

    let hydration = if let Some(path) = &args.sym_path {
        let sym = fs::read_to_string(path).map_err(CliError::Io)?;
        SessionSymbolTable::from_sym_text(&sym).map_err(CliError::SymbolHydration)?
    } else {
        crate::session::SymbolHydration {
            table: SessionSymbolTable::default(),
            warnings: Vec::new(),
        }
    };

    let snapshot = emulator.snapshot().map_err(CliError::Emulator)?;
    let session = Session {
        schema_version: SCHEMA_VERSION,
        parent_sha256: None,
        rom_sha256,
        rom: RomBlob(rom),
        emulator_snapshot: EmulatorSnapshotBlob(snapshot),
        symbols: hydration.table,
        breakpoints: Vec::new(),
        watchpoints: Vec::new(),
        trace_ring: TraceRing::new(args.trace_capacity),
        metadata: SessionMetadata {
            abi_version_observed: None,
            created_at_micros_since_init: 0,
            notes: BTreeMap::new(),
        },
    };
    let session_sha256 = write_session(&session, &args.out_path, args.replace_existing_out)?;
    Ok(InitEnvelope {
        command: "init",
        session_path: path_string(&args.out_path),
        session_sha256: hex_hash(session_sha256),
        rom_sha256: hex_hash(rom_sha256),
        symbol_count: session.symbols.entries.len() as u32,
        warnings: hydration.warnings,
    })
}

pub fn run_exec(args: ExecArgs) -> Result<ExecEnvelope, CliError> {
    if args.in_path == args.out_path {
        return Err(CliError::InOutSamePath);
    }
    let started = Instant::now();
    let input_bytes = fs::read(&args.in_path).map_err(CliError::Io)?;
    let parent_sha256 = sha256_bytes(&input_bytes);
    let session = Session::load_bytes(&input_bytes).map_err(CliError::SessionLoad)?;
    let mut emulator = load_emulator(&session.rom.0)?;
    emulator
        .restore(&session.emulator_snapshot.0)
        .map_err(CliError::Emulator)?;

    let mut script_session = ScriptSession::new(ScriptSessionInput {
        emulator,
        rom: session.rom.0,
        symbols: session.symbols,
        breakpoints: session.breakpoints,
        watchpoints: session.watchpoints,
        trace_ring: session.trace_ring,
        metadata: session.metadata,
        config: args.config.clone(),
    });
    script_session
        .register_persisted_traps()
        .map_err(|error| CliError::ScriptError {
            error: Box::new(error),
            session_path: None,
            partial_session_path: None,
            partial_session_sha256: None,
            determinism: None,
        })?;

    let script = fs::read_to_string(&args.script_path).map_err(CliError::Io)?;
    let host = ScriptHost::new(args.config.clone());
    match host.evaluate(&script, script_session) {
        Ok(success) => {
            let script_micros = started.elapsed().as_micros() as u64;
            let outcome = success.outcome;
            let write_started = Instant::now();
            let session = output_session(success.session, Some(parent_sha256))?;
            let session_sha256 =
                write_session(&session, &args.out_path, args.replace_existing_out)?;
            let metrics = args.emit_metrics.then(|| ExecMetrics {
                script_micros,
                host_setup_micros: 0,
                session_write_micros: write_started.elapsed().as_micros() as u64,
            });
            Ok(ExecEnvelope {
                command: "exec",
                result: outcome.result,
                logs: outcome.logs,
                session_path: path_string(&args.out_path),
                session_sha256: hex_hash(session_sha256),
                parent_sha256: Some(hex_hash(parent_sha256)),
                warnings: outcome.warnings,
                metrics,
            })
        }
        Err(failure) => handle_script_failure(args, parent_sha256, failure),
    }
}

pub fn run_inspect(args: InspectArgs) -> Result<InspectEnvelope, CliError> {
    let input_bytes = fs::read(&args.in_path).map_err(CliError::Io)?;
    let session_sha256 = sha256_bytes(&input_bytes);
    let session = Session::load_bytes(&input_bytes).map_err(CliError::SessionLoad)?;
    let mut emulator = load_emulator(&session.rom.0)?;
    emulator
        .restore(&session.emulator_snapshot.0)
        .map_err(CliError::Emulator)?;
    let (count, banked, unbanked) = session.symbols.summary();
    Ok(InspectEnvelope {
        command: "inspect",
        session_path: path_string(&args.in_path),
        session_sha256: hex_hash(session_sha256),
        schema_version: session.schema_version,
        parent_sha256: session.parent_sha256.map(hex_hash),
        rom_sha256: hex_hash(session.rom_sha256),
        regs: inspect_regs(emulator.regs()),
        breakpoints: session.breakpoints,
        watchpoints: session.watchpoints,
        trace_ring_summary: TraceRingSummary {
            capacity: session.trace_ring.capacity,
            event_count: session.trace_ring.events.len() as u32,
            dropped: session.trace_ring.dropped,
            head_seq: session.trace_ring.events.front().map(|event| event.seq),
            tail_seq: session.trace_ring.events.back().map(|event| event.seq),
        },
        symbols_summary: SymbolsSummary {
            count,
            banked_count: banked,
            unbanked_count: unbanked,
        },
        metadata: session.metadata,
    })
}

fn handle_script_failure(
    args: ExecArgs,
    parent_sha256: [u8; 32],
    failure: Box<ScriptFailure>,
) -> Result<ExecEnvelope, CliError> {
    let is_timeout = matches!(failure.error, ScriptError::Timeout { .. });
    if is_timeout && !args.write_partial_on_timeout {
        return Err(CliError::ScriptError {
            error: Box::new(failure.error),
            session_path: None,
            partial_session_path: None,
            partial_session_sha256: None,
            determinism: None,
        });
    }
    let session = output_session(failure.session, Some(parent_sha256))?;
    let sha = write_session(&session, &args.out_path, args.replace_existing_out)?;
    let session_path = path_string(&args.out_path);
    Err(CliError::ScriptError {
        error: Box::new(failure.error),
        session_path: (!is_timeout).then_some(session_path.clone()),
        partial_session_path: is_timeout.then_some(session_path),
        partial_session_sha256: is_timeout.then(|| hex_hash(sha)),
        determinism: is_timeout.then(|| "nondeterministic_partial".to_owned()),
    })
}

fn output_session(
    mut script_session: ScriptSession,
    parent_sha256: Option<[u8; 32]>,
) -> Result<Session, CliError> {
    script_session.emulator.traps().clear();
    let snapshot = script_session
        .emulator
        .snapshot()
        .map_err(CliError::Emulator)?;
    let rom_sha256 = snapshot.lineage.rom_sha256.to_bytes();
    let mut metadata = script_session.metadata;
    metadata.created_at_micros_since_init = script_session
        .emulator
        .clock_count()
        .0
        .saturating_mul(1_000_000)
        / 4_194_304;
    Ok(Session {
        schema_version: SCHEMA_VERSION,
        parent_sha256,
        rom_sha256,
        rom: RomBlob(script_session.rom),
        emulator_snapshot: EmulatorSnapshotBlob(snapshot),
        symbols: script_session.symbols,
        breakpoints: script_session.breakpoints,
        watchpoints: script_session.watchpoints,
        trace_ring: script_session.trace_ring,
        metadata,
    })
}

fn load_emulator(rom: &[u8]) -> Result<Emulator, CliError> {
    Emulator::load_rom(
        rom,
        EmulatorConfig {
            policy: DeterminismPolicy::default(),
            boot_mode: BootMode::PostBootDmg,
            ..EmulatorConfig::default()
        },
    )
    .map_err(CliError::Emulator)
}

fn write_session(
    session: &Session,
    path: &PathBuf,
    replace_existing: bool,
) -> Result<[u8; 32], CliError> {
    if replace_existing {
        session.replace(path).map_err(CliError::SessionWrite)
    } else {
        session.write_new(path).map_err(CliError::SessionWrite)
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn inspect_regs(regs: gbf_emu::Regs) -> RegsSnapshot {
    RegsSnapshot {
        pc: regs.pc,
        sp: regs.sp,
        a: regs.a,
        b: regs.b,
        c: regs.c,
        d: regs.d,
        e: regs.e,
        h: regs.h,
        l: regs.l,
        f: regs.f.bits(),
        ime: match regs.ime {
            ImeSnapshot::Disabled => "disabled",
            ImeSnapshot::Enabled => "enabled",
            ImeSnapshot::ToBeEnable => "to_be_enable",
        },
    }
}

#[derive(Debug)]
pub enum CliError {
    Io(std::io::Error),
    SessionLoad(SessionLoadError),
    SessionWrite(SessionWriteError),
    SymbolHydration(crate::session::SymbolHydrationError),
    ScriptError {
        error: Box<ScriptError>,
        session_path: Option<String>,
        partial_session_path: Option<String>,
        partial_session_sha256: Option<String>,
        determinism: Option<String>,
    },
    PredicateCompileFailed {
        addr: u16,
        source: String,
        error: String,
    },
    PostLoadPcUnexpected {
        observed: u16,
    },
    CliArgs {
        message: String,
    },
    InOutSamePath,
    Emulator(gbf_emu::EmuError),
}

impl CliError {
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::CliArgs { .. } | Self::InOutSamePath => 1,
            Self::SessionLoad(_) => 2,
            Self::SessionWrite(_) => 3,
            Self::ScriptError { .. } => 4,
            Self::PredicateCompileFailed { .. } => 5,
            Self::Io(_) | Self::PostLoadPcUnexpected { .. } | Self::Emulator(_) => 6,
            Self::SymbolHydration(_) => 7,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "I/O failed: {error}"),
            Self::SessionLoad(error) => write!(f, "{error}"),
            Self::SessionWrite(error) => write!(f, "{error}"),
            Self::SymbolHydration(error) => write!(f, "{error}"),
            Self::ScriptError { error, .. } => write!(f, "{error}"),
            Self::PredicateCompileFailed {
                addr,
                source,
                error,
            } => write!(
                f,
                "predicate at ${addr:04X} failed to compile ({source:?}): {error}"
            ),
            Self::PostLoadPcUnexpected { observed } => {
                write!(f, "post-load PC was ${observed:04X}, expected $0100")
            }
            Self::CliArgs { message } => f.write_str(message),
            Self::InOutSamePath => f.write_str("--in and --out must be different paths"),
            Self::Emulator(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CliError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_error_envelope_serializes() {
        let envelope = ErrorEnvelope::cli_args("bad args".to_owned());
        let json = serde_json::to_value(envelope).expect("serialize");
        assert_eq!(json["kind"], "cli_args");
    }

    #[test]
    fn default_envelope_contains_no_host_timing() {
        let envelope = ExecEnvelope {
            command: "exec",
            result: serde_json::Value::Null,
            logs: Vec::new(),
            session_path: "next.gbsess".to_owned(),
            session_sha256: "00".repeat(32),
            parent_sha256: None,
            warnings: Vec::new(),
            metrics: None,
        };
        let json = serde_json::to_value(envelope).expect("serialize");
        assert!(json["metrics"].is_null());
    }
}
