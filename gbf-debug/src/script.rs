#![forbid(unsafe_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gbf_emu::{
    AddressRange, BreakpointId, ClockCycles, CpuIdleState, CycleBudget, Emulator, ImeSnapshot,
    MCycles, Predicate, RunOutcome as EmuRunOutcome, Snapshot, StepOutcome as EmuStepOutcome,
    TrapAction, TrapKind,
};
use gbf_hw::joypad::Button;
use rquickjs::function::Opt;
use rquickjs::prelude::Func;
use rquickjs::{
    CatchResultExt, Context, Ctx, Error as JsError, Exception, FromJs, IntoJs, Object, Runtime,
    TypedArray, Value,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value as JsonValue};

use crate::session::{
    BreakpointPersisted, PersistedPredicate, SessionMetadata, SessionSymbolEntry,
    SessionSymbolTable, SymbolResolutionError, TraceEventPersisted, TraceRing, WatchpointKind,
    WatchpointPersisted,
};

const DMG_CLOCKS_PER_MICRO: u64 = 4_194_304;

#[derive(Debug, Clone)]
pub struct ScriptConfig {
    pub timeout: Duration,
    pub memory_limit_bytes: Option<usize>,
    pub stack_limit_bytes: Option<usize>,
    pub snapshot_limit: u32,
    pub default_run_budget: CycleBudget,
    pub max_step_instructions_per_call: u32,
}

impl Default for ScriptConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(30),
            memory_limit_bytes: Some(64 * 1024 * 1024),
            stack_limit_bytes: Some(1024 * 1024),
            snapshot_limit: 32,
            default_run_budget: CycleBudget::Machine(MCycles(1_000_000)),
            max_step_instructions_per_call: 1_000_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub kind: String,
    pub detail: JsonValue,
}

impl Warning {
    #[must_use]
    pub fn new(kind: impl Into<String>, detail: JsonValue) -> Self {
        Self {
            kind: kind.into(),
            detail: canonical_json(detail),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogEntry {
    pub message: String,
    pub data: JsonValue,
    pub ts_micros_since_script_start: u64,
}

pub struct ScriptSession {
    pub emulator: Emulator,
    pub rom: Vec<u8>,
    pub symbols: SessionSymbolTable,
    pub breakpoints: Vec<BreakpointPersisted>,
    pub watchpoints: Vec<WatchpointPersisted>,
    pub trace_ring: TraceRing,
    pub metadata: SessionMetadata,
    pub config: ScriptConfig,
    pub logs: Vec<LogEntry>,
    pub warnings: Vec<Warning>,
    snapshots: BTreeMap<u32, Snapshot>,
    next_snapshot_id: u32,
    active_breakpoints: BTreeMap<u16, BreakpointId>,
    active_watchpoints: BTreeMap<(u16, WatchpointKind), BreakpointId>,
    invocation_only_breakpoints: BTreeMap<u16, BreakpointId>,
    invocation_only_watchpoints: BTreeMap<(u16, WatchpointKind), BreakpointId>,
    active_predicates: BTreeMap<u32, RuntimePredicate>,
    next_closure_predicate_id: u32,
    invocation_start_clock: ClockCycles,
    deadline: Instant,
}

pub struct ScriptSessionInput {
    pub emulator: Emulator,
    pub rom: Vec<u8>,
    pub symbols: SessionSymbolTable,
    pub breakpoints: Vec<BreakpointPersisted>,
    pub watchpoints: Vec<WatchpointPersisted>,
    pub trace_ring: TraceRing,
    pub metadata: SessionMetadata,
    pub config: ScriptConfig,
}

impl fmt::Debug for ScriptSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScriptSession")
            .field("rom_len", &self.rom.len())
            .field("symbols", &self.symbols.entries.len())
            .field("breakpoints", &self.breakpoints.len())
            .field("watchpoints", &self.watchpoints.len())
            .field("trace_events", &self.trace_ring.events.len())
            .finish_non_exhaustive()
    }
}

impl ScriptSession {
    pub fn new(input: ScriptSessionInput) -> Self {
        let ScriptSessionInput {
            emulator,
            rom,
            symbols,
            breakpoints,
            watchpoints,
            trace_ring,
            metadata,
            config,
        } = input;
        let invocation_start_clock = emulator.clock_count();
        let deadline = Instant::now() + config.timeout;
        Self {
            emulator,
            rom,
            symbols,
            breakpoints,
            watchpoints,
            trace_ring,
            metadata,
            config,
            logs: Vec::new(),
            warnings: Vec::new(),
            snapshots: BTreeMap::new(),
            next_snapshot_id: 0,
            active_breakpoints: BTreeMap::new(),
            active_watchpoints: BTreeMap::new(),
            invocation_only_breakpoints: BTreeMap::new(),
            invocation_only_watchpoints: BTreeMap::new(),
            active_predicates: BTreeMap::new(),
            next_closure_predicate_id: 0,
            invocation_start_clock,
            deadline,
        }
    }

    pub fn register_persisted_traps(&mut self) -> Result<(), ScriptError> {
        for bp in self.breakpoints.clone().into_iter().filter(|bp| bp.enabled) {
            let id =
                self.emulator
                    .traps()
                    .add_pc(bp.addr, Predicate::Always, TrapAction::HaltAndReport);
            self.active_breakpoints.insert(bp.addr, id);
            self.active_predicates
                .insert(id.0, RuntimePredicate::from_persisted(&bp.predicate));
        }
        for wp in self.watchpoints.clone().into_iter().filter(|wp| wp.enabled) {
            let range = AddressRange::new(wp.addr, wp.addr).map_err(|error| {
                ScriptError::HostBindingError {
                    method: "gb.add_watchpoint".to_owned(),
                    source: error.to_string(),
                }
            })?;
            let id = match wp.kind {
                WatchpointKind::Read => self.emulator.traps().add_mem_read(
                    range,
                    Predicate::Always,
                    TrapAction::HaltAndReport,
                ),
                WatchpointKind::Write => self.emulator.traps().add_mem_write(
                    range,
                    Predicate::Always,
                    TrapAction::HaltAndReport,
                ),
                WatchpointKind::ReadWrite => self.emulator.traps().add_mem_rw(
                    range,
                    Predicate::Always,
                    TrapAction::HaltAndReport,
                ),
            };
            self.active_watchpoints.insert((wp.addr, wp.kind), id);
            self.active_predicates
                .insert(id.0, RuntimePredicate::from_persisted(&wp.predicate));
        }
        Ok(())
    }

    fn virtual_micros(&self) -> u64 {
        self.emulator
            .clock_count()
            .0
            .saturating_sub(self.invocation_start_clock.0)
            .saturating_mul(1_000_000)
            / DMG_CLOCKS_PER_MICRO
    }

    fn drain_trace(&mut self) {
        let pc = self.emulator.regs().pc;
        let events = self.emulator.drain_trace();
        self.trace_ring.extend_normalized(events, pc);
    }

    fn install_breakpoint(
        &mut self,
        addr: u16,
        persist: Option<PersistedPredicate>,
        runtime: RuntimePredicate,
    ) {
        if let Some(old_id) = self.active_breakpoints.remove(&addr) {
            self.emulator.traps().remove(old_id);
            self.active_predicates.remove(&old_id.0);
        }
        let id = self
            .emulator
            .traps()
            .add_pc(addr, Predicate::Always, TrapAction::HaltAndReport);
        self.active_breakpoints.insert(addr, id);
        self.active_predicates.insert(id.0, runtime);
        match persist {
            Some(predicate) => {
                self.invocation_only_breakpoints.remove(&addr);
                self.breakpoints.retain(|bp| bp.addr != addr);
                self.breakpoints.push(BreakpointPersisted {
                    addr,
                    predicate,
                    enabled: true,
                });
                self.breakpoints.sort_by_key(|bp| bp.addr);
            }
            None => {
                self.breakpoints.retain(|bp| bp.addr != addr);
                self.invocation_only_breakpoints.insert(addr, id);
            }
        }
    }

    fn install_watchpoint(
        &mut self,
        addr: u16,
        kind: WatchpointKind,
        persist: Option<PersistedPredicate>,
        runtime: RuntimePredicate,
    ) -> rquickjs::Result<()> {
        if let Some(old_id) = self.active_watchpoints.remove(&(addr, kind)) {
            self.emulator.traps().remove(old_id);
            self.active_predicates.remove(&old_id.0);
        }
        let range = AddressRange::new(addr, addr)
            .map_err(|error| host_error("gb.add_watchpoint", error.to_string()))?;
        let id = match kind {
            WatchpointKind::Read => self.emulator.traps().add_mem_read(
                range,
                Predicate::Always,
                TrapAction::HaltAndReport,
            ),
            WatchpointKind::Write => self.emulator.traps().add_mem_write(
                range,
                Predicate::Always,
                TrapAction::HaltAndReport,
            ),
            WatchpointKind::ReadWrite => self.emulator.traps().add_mem_rw(
                range,
                Predicate::Always,
                TrapAction::HaltAndReport,
            ),
        };
        self.active_watchpoints.insert((addr, kind), id);
        self.active_predicates.insert(id.0, runtime);
        match persist {
            Some(predicate) => {
                self.invocation_only_watchpoints.remove(&(addr, kind));
                self.watchpoints
                    .retain(|wp| !(wp.addr == addr && wp.kind == kind));
                self.watchpoints.push(WatchpointPersisted {
                    addr,
                    kind,
                    predicate,
                    enabled: true,
                });
                self.watchpoints.sort_by_key(|wp| (wp.addr, wp.kind));
            }
            None => {
                self.watchpoints
                    .retain(|wp| !(wp.addr == addr && wp.kind == kind));
                self.invocation_only_watchpoints.insert((addr, kind), id);
            }
        }
        Ok(())
    }

    fn remove_breakpoint(&mut self, addr: u16) {
        self.breakpoints.retain(|bp| bp.addr != addr);
        self.invocation_only_breakpoints.remove(&addr);
        if let Some(id) = self.active_breakpoints.remove(&addr) {
            self.emulator.traps().remove(id);
            self.active_predicates.remove(&id.0);
        }
    }

    fn reenable_pc_trap(&mut self, addr: u16, runtime: RuntimePredicate) {
        let id = self
            .emulator
            .traps()
            .add_pc(addr, Predicate::Always, TrapAction::HaltAndReport);
        self.active_breakpoints.insert(addr, id);
        self.active_predicates.insert(id.0, runtime);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimePredicate {
    Always,
    Source(String),
    ClosureKey(u32),
}

impl RuntimePredicate {
    fn from_persisted(predicate: &PersistedPredicate) -> Self {
        match predicate {
            PersistedPredicate::None => Self::Always,
            PersistedPredicate::StringifiedSource(source) => Self::Source(source.clone()),
        }
    }
}

struct PredicateRegistration {
    persist: Option<PersistedPredicate>,
    runtime: RuntimePredicate,
}

#[derive(Debug, Clone, Serialize)]
struct PredicateAccessJs {
    addr: u16,
    kind: &'static str,
}

#[derive(Debug, Clone)]
struct PredicateScope {
    regs: RegsSnapshotJs,
    pc: u16,
    access: Option<PredicateAccessJs>,
    cycle: u64,
    symbols: SessionSymbolTable,
}

struct PredicateGlobalRestore<'js> {
    values: Vec<(&'static str, Value<'js>)>,
}

impl<'js> PredicateGlobalRestore<'js> {
    fn restore(self, ctx: &Ctx<'js>) -> rquickjs::Result<()> {
        let globals = ctx.globals();
        for (name, value) in self.values {
            globals.set(name, value)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptOutcome {
    pub result: JsonValue,
    pub logs: Vec<LogEntry>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug)]
pub struct ScriptSuccess {
    pub outcome: ScriptOutcome,
    pub session: ScriptSession,
}

#[derive(Debug)]
pub struct ScriptFailure {
    pub error: ScriptError,
    pub session: ScriptSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    SyntaxError {
        message: String,
        line: u32,
        column: u32,
    },
    RuntimeException {
        message: String,
        line: Option<u32>,
        column: Option<u32>,
        function: Option<String>,
    },
    Timeout {
        elapsed_micros: u64,
    },
    OutOfMemory,
    StackOverflow,
    HostBindingError {
        method: String,
        source: String,
    },
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SyntaxError { message, .. } => write!(f, "script syntax error: {message}"),
            Self::RuntimeException { message, .. } => write!(f, "script exception: {message}"),
            Self::Timeout { elapsed_micros } => {
                write!(f, "script timed out after {elapsed_micros} us")
            }
            Self::OutOfMemory => f.write_str("script ran out of memory"),
            Self::StackOverflow => f.write_str("script stack overflow"),
            Self::HostBindingError { method, source } => write!(f, "{method} failed: {source}"),
        }
    }
}

impl std::error::Error for ScriptError {}

#[derive(Debug, Clone)]
pub struct ScriptHost {
    config: ScriptConfig,
}

impl ScriptHost {
    #[must_use]
    pub fn new(config: ScriptConfig) -> Self {
        Self { config }
    }

    pub fn evaluate(
        &self,
        script_source: &str,
        session: ScriptSession,
    ) -> Result<ScriptSuccess, Box<ScriptFailure>> {
        let started = Instant::now();
        let runtime = match Runtime::new() {
            Ok(runtime) => runtime,
            Err(error) => return Err(failure(session, map_js_error(error, started))),
        };
        if let Some(limit) = self.config.memory_limit_bytes {
            runtime.set_memory_limit(limit);
        }
        if let Some(limit) = self.config.stack_limit_bytes {
            runtime.set_max_stack_size(limit);
        }
        let deadline = Instant::now() + self.config.timeout;
        runtime.set_interrupt_handler(Some(Box::new(move || Instant::now() >= deadline)));
        let context = match Context::full(&runtime) {
            Ok(context) => context,
            Err(error) => return Err(failure(session, map_js_error(error, started))),
        };

        let state = Rc::new(RefCell::new(Some(session)));
        let eval_result = context.with(|ctx| -> rquickjs::Result<_> {
            install_deterministic_globals(ctx.clone(), state.clone())?;
            install_gb(ctx.clone(), state.clone())?;
            validate_runtime_predicates(&ctx, state.clone())?;
            ctx.globals().set("result", rquickjs::Null)?;
            ctx.eval::<(), _>(GB_WRAPPER)?;
            let run = ctx
                .eval::<Value<'_>, _>(script_source)
                .catch(&ctx)
                .map(|_| ())
                .map_err(|error| map_caught_or_timeout(error, started, self.config.timeout));
            Ok(run)
        });

        match eval_result {
            Ok(Ok(())) => {
                let (result, mut result_warnings) = context
                    .with(|ctx| {
                        let result_value: Value<'_> = ctx.globals().get("result")?;
                        Ok::<_, rquickjs::Error>(js_value_to_canonical_json(result_value))
                    })
                    .unwrap_or_else(|error| {
                        (
                            JsonValue::Null,
                            vec![Warning::new(
                                "result_not_serializable",
                                serde_json::json!({ "error": error.to_string() }),
                            )],
                        )
                    });
                let mut session = state.borrow_mut().take().expect("script session present");
                session.warnings.append(&mut result_warnings);
                let outcome = ScriptOutcome {
                    result,
                    logs: session.logs.clone(),
                    warnings: session.warnings.clone(),
                };
                Ok(ScriptSuccess { outcome, session })
            }
            Ok(Err(error)) => {
                let session = state.borrow_mut().take().expect("script session present");
                Err(Box::new(ScriptFailure { error, session }))
            }
            Err(error) => {
                let session = state.borrow_mut().take().expect("script session present");
                Err(Box::new(ScriptFailure {
                    error: map_js_error(error, started),
                    session,
                }))
            }
        }
    }
}

fn failure(session: ScriptSession, error: ScriptError) -> Box<ScriptFailure> {
    Box::new(ScriptFailure { error, session })
}

fn install_deterministic_globals<'js>(
    ctx: Ctx<'js>,
    state: Rc<RefCell<Option<ScriptSession>>>,
) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    globals.set("__gb_closure_predicates", Object::new(ctx.clone())?)?;
    let date = Object::new(ctx.clone())?;
    let date_state = state.clone();
    date.set(
        "now",
        Func::from(move || -> rquickjs::Result<u64> {
            let state = date_state.borrow();
            let session = state.as_ref().expect("script session present");
            Ok(session.virtual_micros() / 1_000)
        }),
    )?;
    globals.set("Date", date)?;

    let math: Object<'_> = globals.get("Math")?;
    let random_state = Rc::new(RefCell::new(0xDEAD_BEEF_CAFE_BABE_u64));
    math.set(
        "random",
        Func::from(move || -> f64 {
            let mut seed = random_state.borrow_mut();
            let mut x = *seed;
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            *seed = x;
            let value = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
            (value >> 11) as f64 / ((1_u64 << 53) as f64)
        }),
    )?;
    let _ = globals.remove("console");

    let log_state = state.clone();
    globals.set(
        "log",
        Func::from(
            move |message: String, data: Opt<Value<'js>>| -> rquickjs::Result<()> {
                let (data, mut warnings) = match data.0 {
                    Some(value) => js_value_to_canonical_json(value),
                    None => (JsonValue::Null, Vec::new()),
                };
                let mut state = log_state.borrow_mut();
                let session = state.as_mut().expect("script session present");
                session.warnings.append(&mut warnings);
                session.logs.push(LogEntry {
                    message,
                    data,
                    ts_micros_since_script_start: session.virtual_micros(),
                });
                Ok(())
            },
        ),
    )?;

    Ok(())
}

fn install_gb<'js>(
    ctx: Ctx<'js>,
    state: Rc<RefCell<Option<ScriptSession>>>,
) -> rquickjs::Result<()> {
    let gb = Object::new(ctx.clone())?;
    ctx.globals().set("gb", gb.clone())?;

    let s = state.clone();
    gb.set(
        "_regs",
        Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            to_js(ctx, regs_snapshot(session.emulator.regs()))
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_read",
        Func::from(move |addr: u32, len: u32| -> rquickjs::Result<Vec<u8>> {
            let (addr, len) = checked_range("gb.read", addr, len)?;
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            session
                .emulator
                .peek_range(addr, len)
                .map_err(|error| host_error("gb.read", error.to_string()))
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_write",
        Func::from(
            move |ctx: Ctx<'js>, addr: u32, bytes: Value<'js>| -> rquickjs::Result<()> {
                let bytes = bytes_from_js(&ctx, bytes)?;
                let (addr, _) = checked_range("gb.write", addr, bytes.len() as u32)?;
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                for (offset, byte) in bytes.into_iter().enumerate() {
                    let addr = addr + u16::try_from(offset).expect("range already checked");
                    session
                        .emulator
                        .poke(addr, byte)
                        .map_err(|error| host_error("gb.write", error.to_string()))?;
                }
                session.drain_trace();
                Ok(())
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_bus_read",
        Func::from(move |addr: u32, len: u32| -> rquickjs::Result<Vec<u8>> {
            let (addr, len) = checked_range("gb.bus_read", addr, len)?;
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            let mut out = Vec::with_capacity(len);
            for offset in 0..len {
                let byte = session
                    .emulator
                    .bus_read(addr + u16::try_from(offset).expect("range checked"))
                    .map_err(|error| host_error("gb.bus_read", error.to_string()))?;
                out.push(byte);
            }
            session.drain_trace();
            Ok(out)
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_bus_write",
        Func::from(
            move |ctx: Ctx<'js>, addr: u32, bytes: Value<'js>| -> rquickjs::Result<()> {
                let bytes = bytes_from_js(&ctx, bytes)?;
                let (addr, _) = checked_range("gb.bus_write", addr, bytes.len() as u32)?;
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                for (offset, byte) in bytes.into_iter().enumerate() {
                    session
                        .emulator
                        .bus_write(addr + u16::try_from(offset).expect("range checked"), byte)
                        .map_err(|error| host_error("gb.bus_write", error.to_string()))?;
                }
                session.drain_trace();
                Ok(())
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_step",
        Func::from(
            move |ctx: Ctx<'js>, n: u32| -> rquickjs::Result<Value<'js>> {
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                if n > session.config.max_step_instructions_per_call {
                    return Err(host_error(
                        "gb.step",
                        format!(
                            "requested {n} instructions, cap is {}",
                            session.config.max_step_instructions_per_call
                        ),
                    ));
                }
                let start = session.emulator.clock_count();
                for _ in 0..n {
                    if Instant::now() >= session.deadline {
                        return Err(Exception::throw_message(
                            &ctx,
                            "gb.step exceeded script timeout",
                        ));
                    }
                    match session
                        .emulator
                        .step()
                        .map_err(|error| host_error("gb.step", error.to_string()))?
                    {
                        EmuStepOutcome::Stepped { .. } => {}
                        EmuStepOutcome::TrapHit { .. } | EmuStepOutcome::Idle { .. } => break,
                    }
                }
                let end = session.emulator.clock_count();
                session.drain_trace();
                session.trace_ring.push(TraceEventPersisted::step_boundary(
                    session.emulator.regs().pc,
                ));
                to_js(
                    ctx,
                    step_outcome(session.emulator.regs().pc, cycles_between(start, end)),
                )
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_run_until",
        Func::from(
            move |ctx: Ctx<'js>, pc: u32, max_m_cycles: Opt<u64>| -> rquickjs::Result<Value<'js>> {
                let pc =
                    u16::try_from(pc).map_err(|_| host_error("gb.run_until", "pc out of range"))?;
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                let budget = budget_or_default(max_m_cycles.0, session.config.default_run_budget);
                let start = session.emulator.clock_count();
                let outcome = session
                    .emulator
                    .run_until_pc(pc, budget)
                    .map_err(|error| host_error("gb.run_until", error.to_string()))?;
                let end = session.emulator.clock_count();
                session.drain_trace();
                to_js(
                    ctx,
                    run_outcome(
                        outcome,
                        session.emulator.regs().pc,
                        cycles_between(start, end),
                    ),
                )
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_run_until_breakpoint",
        Func::from(
            move |ctx: Ctx<'js>, max_m_cycles: Opt<u64>| -> rquickjs::Result<Value<'js>> {
                let (requested, start) = {
                    let state = s.borrow();
                    let session = state.as_ref().expect("script session present");
                    (
                        budget_or_default(max_m_cycles.0, session.config.default_run_budget)
                            .as_clock_cycles(),
                        session.emulator.clock_count(),
                    )
                };
                let target = ClockCycles(start.0.saturating_add(requested.0));
                loop {
                    let trap = {
                        let mut state = s.borrow_mut();
                        let session = state.as_mut().expect("script session present");
                        let now = session.emulator.clock_count();
                        if now.0 >= target.0 {
                            let outcome = EmuRunOutcome::BudgetElapsed {
                                observed: now,
                                requested,
                            };
                            return to_js(
                                ctx,
                                run_outcome(
                                    outcome,
                                    session.emulator.regs().pc,
                                    cycles_between(start, now),
                                ),
                            );
                        }
                        let remaining = ClockCycles(target.0.saturating_sub(now.0));
                        let outcome = session
                            .emulator
                            .run_for(CycleBudget::Clock(remaining))
                            .map_err(|error| {
                                host_error("gb.run_until_breakpoint", error.to_string())
                            })?;
                        session.drain_trace();
                        if let EmuRunOutcome::TrapHit { trap_id, kind, .. } = outcome {
                            let predicate = session
                                .active_predicates
                                .get(&trap_id.0)
                                .cloned()
                                .unwrap_or(RuntimePredicate::Always);
                            let scope = predicate_scope(session, kind);
                            let end = session.emulator.clock_count();
                            Some((
                                outcome,
                                trap_id,
                                kind,
                                predicate,
                                scope,
                                session.emulator.regs().pc,
                                cycles_between(start, end),
                            ))
                        } else {
                            let end = session.emulator.clock_count();
                            return to_js(
                                ctx,
                                run_outcome(
                                    outcome,
                                    session.emulator.regs().pc,
                                    cycles_between(start, end),
                                ),
                            );
                        }
                    };

                    let (outcome, trap_id, kind, predicate, scope, pc_at_stop, cycles_consumed) =
                        trap.expect("trap branch returns Some");
                    if evaluate_runtime_predicate(&ctx, predicate, scope)? {
                        return to_js(ctx, run_outcome(outcome, pc_at_stop, cycles_consumed));
                    }
                    if let TrapKind::Pc { addr } = kind {
                        let step_trap = {
                            let mut state = s.borrow_mut();
                            let session = state.as_mut().expect("script session present");
                            let runtime = session
                                .active_predicates
                                .remove(&trap_id.0)
                                .unwrap_or(RuntimePredicate::Always);
                            session.emulator.traps().remove(trap_id);
                            session.active_breakpoints.remove(&addr);
                            let step_outcome = session.emulator.step().map_err(|error| {
                                host_error("gb.run_until_breakpoint", error.to_string())
                            })?;
                            session.drain_trace();
                            session.reenable_pc_trap(addr, runtime);
                            if let EmuStepOutcome::TrapHit { trap_id, kind, .. } = step_outcome {
                                let predicate = session
                                    .active_predicates
                                    .get(&trap_id.0)
                                    .cloned()
                                    .unwrap_or(RuntimePredicate::Always);
                                let scope = predicate_scope(session, kind);
                                let end = session.emulator.clock_count();
                                Some((
                                    EmuRunOutcome::TrapHit {
                                        trap_id,
                                        kind,
                                        observed: end,
                                    },
                                    predicate,
                                    scope,
                                    session.emulator.regs().pc,
                                    cycles_between(start, end),
                                ))
                            } else {
                                None
                            }
                        };
                        if let Some((outcome, predicate, scope, pc_at_stop, cycles_consumed)) =
                            step_trap
                            && evaluate_runtime_predicate(&ctx, predicate, scope)?
                        {
                            return to_js(ctx, run_outcome(outcome, pc_at_stop, cycles_consumed));
                        }
                    }
                }
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_add_breakpoint",
        Func::from(
            move |addr: u32, predicate: Opt<Value<'js>>| -> rquickjs::Result<()> {
                let addr = u16::try_from(addr)
                    .map_err(|_| host_error("gb.add_breakpoint", "addr out of range"))?;
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                let registration =
                    predicate_to_registration("gb.add_breakpoint", predicate.0, session)?;
                session.install_breakpoint(addr, registration.persist, registration.runtime);
                Ok(())
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_remove_breakpoint",
        Func::from(move |addr: u32| -> rquickjs::Result<()> {
            let addr = u16::try_from(addr)
                .map_err(|_| host_error("gb.remove_breakpoint", "addr out of range"))?;
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            session.remove_breakpoint(addr);
            Ok(())
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_list_breakpoints",
        Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            let mut list: Vec<_> = session
                .breakpoints
                .iter()
                .map(|bp| BreakpointListEntry {
                    addr: bp.addr,
                    has_predicate: !matches!(bp.predicate, PersistedPredicate::None),
                    persisted_kind: match bp.predicate {
                        PersistedPredicate::None => "none",
                        PersistedPredicate::StringifiedSource(_) => "stringified",
                    },
                })
                .collect();
            list.extend(session.invocation_only_breakpoints.keys().map(|addr| {
                BreakpointListEntry {
                    addr: *addr,
                    has_predicate: true,
                    persisted_kind: "none",
                }
            }));
            list.sort_by_key(|entry| entry.addr);
            to_js(ctx, list)
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_add_watchpoint",
        Func::from(
            move |addr: u32, kind: String, predicate: Opt<Value<'js>>| -> rquickjs::Result<()> {
                let addr = u16::try_from(addr)
                    .map_err(|_| host_error("gb.add_watchpoint", "addr out of range"))?;
                let kind = WatchpointKind::parse(&kind)
                    .ok_or_else(|| host_error("gb.add_watchpoint", "unknown watchpoint kind"))?;
                let mut state = s.borrow_mut();
                let session = state.as_mut().expect("script session present");
                let registration =
                    predicate_to_registration("gb.add_watchpoint", predicate.0, session)?;
                session.install_watchpoint(addr, kind, registration.persist, registration.runtime)
            },
        ),
    )?;

    let s = state.clone();
    gb.set(
        "_remove_watchpoint",
        Func::from(move |addr: u32, kind: String| -> rquickjs::Result<()> {
            let addr = u16::try_from(addr)
                .map_err(|_| host_error("gb.remove_watchpoint", "addr out of range"))?;
            let kind = WatchpointKind::parse(&kind)
                .ok_or_else(|| host_error("gb.remove_watchpoint", "unknown watchpoint kind"))?;
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            session
                .watchpoints
                .retain(|wp| !(wp.addr == addr && wp.kind == kind));
            session.invocation_only_watchpoints.remove(&(addr, kind));
            if let Some(id) = session.active_watchpoints.remove(&(addr, kind)) {
                session.emulator.traps().remove(id);
                session.active_predicates.remove(&id.0);
            }
            Ok(())
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_list_watchpoints",
        Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            let mut list: Vec<_> = session
                .watchpoints
                .iter()
                .map(|wp| WatchpointListEntry {
                    addr: wp.addr,
                    kind: wp.kind.as_str(),
                    has_predicate: !matches!(wp.predicate, PersistedPredicate::None),
                    persisted_kind: match wp.predicate {
                        PersistedPredicate::None => "none",
                        PersistedPredicate::StringifiedSource(_) => "stringified",
                    },
                })
                .collect();
            list.extend(
                session
                    .invocation_only_watchpoints
                    .keys()
                    .map(|(addr, kind)| WatchpointListEntry {
                        addr: *addr,
                        kind: kind.as_str(),
                        has_predicate: true,
                        persisted_kind: "none",
                    }),
            );
            list.sort_by_key(|entry| (entry.addr, entry.kind));
            to_js(ctx, list)
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_snapshot",
        Func::from(move || -> rquickjs::Result<u32> {
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            if session.snapshots.len() as u32 >= session.config.snapshot_limit {
                return Err(host_error("gb.snapshot", "snapshot limit exceeded"));
            }
            let id = session.next_snapshot_id;
            session.next_snapshot_id = session.next_snapshot_id.saturating_add(1);
            let snapshot = session
                .emulator
                .snapshot()
                .map_err(|error| host_error("gb.snapshot", error.to_string()))?;
            session.snapshots.insert(id, snapshot);
            Ok(id)
        }),
    )?;

    let s = state.clone();
    gb.set(
        "_restore",
        Func::from(move |handle: u32| -> rquickjs::Result<()> {
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            let snapshot = session
                .snapshots
                .get(&handle)
                .cloned()
                .ok_or_else(|| host_error("gb.restore", "unknown snapshot handle"))?;
            session
                .emulator
                .restore(&snapshot)
                .map_err(|error| host_error("gb.restore", error.to_string()))?;
            Ok(())
        }),
    )?;

    install_symbol_methods(&gb, state.clone())?;
    install_display_methods(&gb, state)?;
    Ok(())
}

fn install_symbol_methods<'js>(
    gb: &Object<'js>,
    state: Rc<RefCell<Option<ScriptSession>>>,
) -> rquickjs::Result<()> {
    let s = state.clone();
    gb.set(
        "_symbol",
        Func::from(move |name: String| -> rquickjs::Result<Option<u16>> {
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            session.symbols.resolve(&name).map_err(symbol_error_to_js)
        }),
    )?;
    let s = state.clone();
    gb.set(
        "_symbol_in_bank",
        Func::from(
            move |name: String, bank: u32| -> rquickjs::Result<Option<u16>> {
                let bank = u16::try_from(bank)
                    .map_err(|_| host_error("gb.symbol_in_bank", "bank out of range"))?;
                let state = s.borrow();
                let session = state.as_ref().expect("script session present");
                Ok(session.symbols.resolve_in_bank(&name, bank))
            },
        ),
    )?;
    let s = state.clone();
    gb.set(
        "_symbol_at",
        Func::from(move |addr: u32| -> rquickjs::Result<Option<String>> {
            let addr =
                u16::try_from(addr).map_err(|_| host_error("gb.symbol_at", "addr out of range"))?;
            let state = s.borrow();
            let session = state.as_ref().expect("script session present");
            session
                .symbols
                .resolve_at(addr)
                .map(|opt| opt.map(str::to_owned))
                .map_err(symbol_error_to_js)
        }),
    )?;
    let s = state;
    gb.set(
        "_symbol_at_in_bank",
        Func::from(
            move |addr: u32, bank: u32| -> rquickjs::Result<Option<String>> {
                let addr = u16::try_from(addr)
                    .map_err(|_| host_error("gb.symbol_at_in_bank", "addr out of range"))?;
                let bank = u16::try_from(bank)
                    .map_err(|_| host_error("gb.symbol_at_in_bank", "bank out of range"))?;
                let state = s.borrow();
                let session = state.as_ref().expect("script session present");
                Ok(session
                    .symbols
                    .resolve_at_in_bank(addr, bank)
                    .map(str::to_owned))
            },
        ),
    )
}

fn install_display_methods<'js>(
    gb: &Object<'js>,
    state: Rc<RefCell<Option<ScriptSession>>>,
) -> rquickjs::Result<()> {
    let s = state.clone();
    gb.set(
        "_framebuffer",
        Func::from(move || -> rquickjs::Result<Vec<u8>> {
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            Ok(session.emulator.framebuffer().as_bytes().to_vec())
        }),
    )?;
    let s = state.clone();
    gb.set(
        "_input",
        Func::from(move |buttons: Vec<String>| -> rquickjs::Result<()> {
            let mut frame = gbf_emu::JoypadFrame::default();
            for button in buttons {
                frame = frame.with(parse_button(&button)?);
            }
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            session.emulator.set_joypad(frame);
            Ok(())
        }),
    )?;
    let s = state.clone();
    gb.set(
        "_trace_ring",
        Func::from(move |ctx: Ctx<'js>| -> rquickjs::Result<Value<'js>> {
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            session.drain_trace();
            let events: Vec<_> = session
                .trace_ring
                .events
                .iter()
                .map(TraceEventJs::from)
                .collect();
            to_js(ctx, events)
        }),
    )?;
    let s = state;
    gb.set(
        "_clear_trace",
        Func::from(move || -> rquickjs::Result<()> {
            let mut state = s.borrow_mut();
            let session = state.as_mut().expect("script session present");
            session.trace_ring.clear();
            Ok(())
        }),
    )
}

fn validate_runtime_predicates<'js>(
    ctx: &Ctx<'js>,
    state: Rc<RefCell<Option<ScriptSession>>>,
) -> rquickjs::Result<()> {
    let state = state.borrow();
    let session = state.as_ref().expect("script session present");
    for predicate in session.active_predicates.values() {
        if let RuntimePredicate::Source(source) = predicate {
            compile_source_predicate(ctx, "predicate", source)?;
        }
    }
    Ok(())
}

const GB_WRAPPER: &str = r#"
Object.defineProperty(gb, "regs", { enumerable: true, get() { return Object.freeze(gb._regs()); } });
gb.read = (addr, len) => new Uint8Array(gb._read(addr, len));
gb.write = (addr, bytes) => gb._write(addr, bytes);
gb.bus_read = (addr, len) => new Uint8Array(gb._bus_read(addr, len));
gb.bus_write = (addr, bytes) => gb._bus_write(addr, bytes);
gb.step = (n = 1) => gb._step(n);
gb.run_until = (pc, max_m_cycles) => gb._run_until(pc, max_m_cycles);
gb.run_until_breakpoint = (max_m_cycles) => gb._run_until_breakpoint(max_m_cycles);
gb.add_breakpoint = (addr, predicate) => gb._add_breakpoint(addr, predicate);
gb.remove_breakpoint = (addr) => gb._remove_breakpoint(addr);
gb.list_breakpoints = () => gb._list_breakpoints();
gb.add_watchpoint = (addr, kind, predicate) => gb._add_watchpoint(addr, kind, predicate);
gb.remove_watchpoint = (addr, kind) => gb._remove_watchpoint(addr, kind);
gb.list_watchpoints = () => gb._list_watchpoints();
gb.snapshot = () => gb._snapshot();
gb.restore = (handle) => gb._restore(handle);
gb.symbol = (name) => { const v = gb._symbol(name); return v === undefined ? null : v; };
gb.symbol_in_bank = (name, bank) => { const v = gb._symbol_in_bank(name, bank); return v === undefined ? null : v; };
gb.symbol_at = (addr) => { const v = gb._symbol_at(addr); return v === undefined ? null : v; };
gb.symbol_at_in_bank = (addr, bank) => { const v = gb._symbol_at_in_bank(addr, bank); return v === undefined ? null : v; };
gb.framebuffer = () => new Uint8Array(gb._framebuffer());
gb.input = (buttons) => gb._input(buttons);
gb.trace_ring = () => gb._trace_ring().map((event) => ({ ...event, data: new Uint8Array(event.data) }));
gb.clear_trace = () => gb._clear_trace();
"#;

#[derive(Debug, Clone, Serialize)]
struct RegsSnapshotJs {
    pc: u16,
    sp: u16,
    a: u8,
    b: u8,
    c: u8,
    d: u8,
    e: u8,
    h: u8,
    l: u8,
    f: u8,
    bc: u16,
    de: u16,
    hl: u16,
    ime: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct StepOutcomeJs {
    pc_after: u16,
    clock_cycles_consumed: String,
    m_cycles_floor_consumed: String,
}

#[derive(Debug, Clone, Serialize)]
struct RunOutcomeJs {
    reason: &'static str,
    pc_at_stop: u16,
    clock_cycles_consumed: String,
    m_cycles_floor_consumed: String,
    trap_id: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct BreakpointListEntry {
    addr: u16,
    has_predicate: bool,
    persisted_kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct WatchpointListEntry {
    addr: u16,
    kind: &'static str,
    has_predicate: bool,
    persisted_kind: &'static str,
}

#[derive(Debug, Clone, Serialize)]
struct TraceEventJs {
    seq: String,
    kind: &'static str,
    addr: u16,
    data: Vec<u8>,
    pc_at: u16,
}

impl From<&TraceEventPersisted> for TraceEventJs {
    fn from(value: &TraceEventPersisted) -> Self {
        Self {
            seq: value.seq.to_string(),
            kind: match value.kind {
                crate::session::TraceEventKind::MemoryWrite => "mem_write",
                crate::session::TraceEventKind::RomBankSwitch => "rom_bank_switch",
                crate::session::TraceEventKind::SramBankSwitch => "sram_bank_switch",
                crate::session::TraceEventKind::IoWrite => "io_write",
                crate::session::TraceEventKind::TrapHit => "trap_hit",
                crate::session::TraceEventKind::Typed => "typed",
                crate::session::TraceEventKind::StepBoundary => "step_boundary",
            },
            addr: value.addr,
            data: value.data.clone(),
            pc_at: value.pc_at,
        }
    }
}

fn regs_snapshot(regs: gbf_emu::Regs) -> RegsSnapshotJs {
    RegsSnapshotJs {
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
        bc: u16::from(regs.b) << 8 | u16::from(regs.c),
        de: u16::from(regs.d) << 8 | u16::from(regs.e),
        hl: u16::from(regs.h) << 8 | u16::from(regs.l),
        ime: ime_str(regs.ime),
    }
}

fn step_outcome(pc_after: u16, cycles: ClockCycles) -> StepOutcomeJs {
    StepOutcomeJs {
        pc_after,
        clock_cycles_consumed: cycles.0.to_string(),
        m_cycles_floor_consumed: cycles.as_m_cycles_floor().0.to_string(),
    }
}

fn run_outcome(outcome: EmuRunOutcome, pc_at_stop: u16, consumed: ClockCycles) -> RunOutcomeJs {
    match outcome {
        EmuRunOutcome::TrapHit { trap_id, kind, .. } => RunOutcomeJs {
            reason: if trap_id == BreakpointId::RUN_UNTIL_PC {
                "pc_reached"
            } else if matches!(kind, TrapKind::Pc { .. }) {
                "breakpoint"
            } else {
                "watchpoint"
            },
            pc_at_stop,
            clock_cycles_consumed: consumed.0.to_string(),
            m_cycles_floor_consumed: consumed.as_m_cycles_floor().0.to_string(),
            trap_id: (trap_id != BreakpointId::RUN_UNTIL_PC).then_some(trap_id.0),
        },
        EmuRunOutcome::BudgetElapsed { .. } => RunOutcomeJs {
            reason: "max_cycles_exceeded",
            pc_at_stop,
            clock_cycles_consumed: consumed.0.to_string(),
            m_cycles_floor_consumed: consumed.as_m_cycles_floor().0.to_string(),
            trap_id: None,
        },
        EmuRunOutcome::Idle { state, .. } => RunOutcomeJs {
            reason: match state {
                CpuIdleState::Halt => "idle_halt",
                CpuIdleState::Stop => "idle_stop",
            },
            pc_at_stop,
            clock_cycles_consumed: consumed.0.to_string(),
            m_cycles_floor_consumed: consumed.as_m_cycles_floor().0.to_string(),
            trap_id: None,
        },
    }
}

fn ime_str(value: ImeSnapshot) -> &'static str {
    match value {
        ImeSnapshot::Disabled => "disabled",
        ImeSnapshot::Enabled => "enabled",
        ImeSnapshot::ToBeEnable => "to_be_enable",
    }
}

fn cycles_between(start: ClockCycles, end: ClockCycles) -> ClockCycles {
    ClockCycles(end.0.saturating_sub(start.0))
}

fn budget_or_default(max_m_cycles: Option<u64>, default: CycleBudget) -> CycleBudget {
    max_m_cycles
        .map(|cycles| CycleBudget::Machine(MCycles(cycles)))
        .unwrap_or(default)
}

fn checked_range(method: &'static str, addr: u32, len: u32) -> rquickjs::Result<(u16, usize)> {
    if addr > 0xFFFF {
        return Err(host_error(method, "address out of range"));
    }
    if addr.saturating_add(len) > 0x1_0000 {
        return Err(host_error(
            method,
            "address range overflows u16 address space",
        ));
    }
    Ok((addr as u16, len as usize))
}

fn bytes_from_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> rquickjs::Result<Vec<u8>> {
    if let Ok(array) = TypedArray::<u8>::from_js(ctx, value.clone()) {
        let slice: &[u8] = array.as_ref();
        return Ok(slice.to_vec());
    }
    Vec::<u8>::from_js(ctx, value).map_err(|error| host_error("bytes", error.to_string()))
}

fn predicate_to_registration<'js>(
    method: &'static str,
    value: Option<Value<'js>>,
    session: &mut ScriptSession,
) -> rquickjs::Result<PredicateRegistration> {
    let Some(value) = value else {
        return Ok(PredicateRegistration {
            persist: Some(PersistedPredicate::None),
            runtime: RuntimePredicate::Always,
        });
    };
    if value.is_undefined() || value.is_null() {
        return Ok(PredicateRegistration {
            persist: Some(PersistedPredicate::None),
            runtime: RuntimePredicate::Always,
        });
    }
    if value.is_string() {
        let ctx = value.ctx().clone();
        let source =
            String::from_js(&ctx, value).map_err(|error| host_error(method, error.to_string()))?;
        compile_source_predicate(&ctx, method, &source)?;
        return Ok(PredicateRegistration {
            persist: Some(PersistedPredicate::StringifiedSource(source.clone())),
            runtime: RuntimePredicate::Source(source),
        });
    }
    if value.is_function() {
        let key = session.next_closure_predicate_id;
        session.next_closure_predicate_id = session.next_closure_predicate_id.saturating_add(1);
        let ctx = value.ctx().clone();
        let store: Object<'_> = ctx.globals().get("__gb_closure_predicates")?;
        store.set(key.to_string(), value)?;
        session.warnings.push(Warning::new(
            "predicate_not_persisted",
            serde_json::json!({
                "method": method,
                "reason": "closure predicates are invocation-local"
            }),
        ));
        return Ok(PredicateRegistration {
            persist: None,
            runtime: RuntimePredicate::ClosureKey(key),
        });
    }
    Err(host_error(
        method,
        "predicate must be a string, function, null, or undefined",
    ))
}

fn compile_source_predicate<'js>(
    ctx: &Ctx<'js>,
    method: &'static str,
    source: &str,
) -> rquickjs::Result<()> {
    let program = source_predicate_program(source);
    ctx.eval::<Value<'_>, _>(program)
        .map(|_| ())
        .map_err(|error| host_error(method, error.to_string()))
}

fn source_predicate_program(source: &str) -> String {
    format!(
        "((regs, pc, access, cycle, symbol, symbolInBank, gb, globalThis) => Boolean(({source})))"
    )
}

fn predicate_scope(session: &ScriptSession, kind: TrapKind) -> PredicateScope {
    PredicateScope {
        regs: regs_snapshot(session.emulator.regs()),
        pc: session.emulator.regs().pc,
        access: predicate_access(kind),
        cycle: session.emulator.clock_count().0,
        symbols: session.symbols.clone(),
    }
}

fn predicate_access(kind: TrapKind) -> Option<PredicateAccessJs> {
    match kind {
        TrapKind::Pc { .. } => None,
        TrapKind::MemRead { range } => Some(PredicateAccessJs {
            addr: range.start(),
            kind: "read",
        }),
        TrapKind::MemWrite { range } => Some(PredicateAccessJs {
            addr: range.start(),
            kind: "write",
        }),
        TrapKind::MemRw { range } => Some(PredicateAccessJs {
            addr: range.start(),
            kind: "rw",
        }),
    }
}

fn evaluate_runtime_predicate<'js>(
    ctx: &Ctx<'js>,
    predicate: RuntimePredicate,
    scope: PredicateScope,
) -> rquickjs::Result<bool> {
    match predicate {
        RuntimePredicate::Always => Ok(true),
        RuntimePredicate::Source(source) => {
            let restore = install_predicate_scope(ctx, scope)?;
            let program = format!(
                "{}(__gb_predicate_regs, __gb_predicate_pc, __gb_predicate_access, __gb_predicate_cycle, __gb_predicate_symbol, __gb_predicate_symbolInBank, undefined, Object.freeze({{}}))",
                source_predicate_program(&source)
            );
            let result = ctx.eval::<bool, _>(program);
            restore.restore(ctx)?;
            result.map_err(|error| host_error("predicate", error.to_string()))
        }
        RuntimePredicate::ClosureKey(key) => {
            let restore = install_predicate_scope(ctx, scope)?;
            let result = ctx.eval::<bool, _>(format!("Boolean(__gb_closure_predicates[{key}]())"));
            restore.restore(ctx)?;
            result.map_err(|error| host_error("predicate", error.to_string()))
        }
    }
}

fn install_predicate_scope<'js>(
    ctx: &Ctx<'js>,
    scope: PredicateScope,
) -> rquickjs::Result<PredicateGlobalRestore<'js>> {
    let globals = ctx.globals();
    let restore = PredicateGlobalRestore {
        values: [
            "regs",
            "pc",
            "access",
            "cycle",
            "symbol",
            "symbolInBank",
            "gb",
        ]
        .into_iter()
        .map(|name| Ok((name, globals.get(name)?)))
        .collect::<rquickjs::Result<_>>()?,
    };

    let regs = to_js(ctx.clone(), scope.regs)?;
    let access = match scope.access {
        Some(access) => to_js(ctx.clone(), access)?,
        None => rquickjs::Null.into_js(ctx)?,
    };
    let cycle = scope.cycle.to_string();

    globals.set("__gb_predicate_regs", regs.clone())?;
    globals.set("__gb_predicate_pc", scope.pc)?;
    globals.set("__gb_predicate_access", access.clone())?;
    globals.set("__gb_predicate_cycle", cycle.clone())?;

    let symbols = scope.symbols.clone();
    globals.set(
        "__gb_predicate_symbol",
        Func::from(move |name: String| -> rquickjs::Result<Option<u16>> {
            symbols.resolve(&name).map_err(symbol_error_to_js)
        }),
    )?;
    let symbols = scope.symbols.clone();
    globals.set(
        "__gb_predicate_symbolInBank",
        Func::from(
            move |name: String, bank: u32| -> rquickjs::Result<Option<u16>> {
                let bank = u16::try_from(bank)
                    .map_err(|_| host_error("predicate.symbolInBank", "bank out of range"))?;
                Ok(symbols.resolve_in_bank(&name, bank))
            },
        ),
    )?;

    globals.set("regs", regs.clone())?;
    globals.set("pc", scope.pc)?;
    globals.set("access", access.clone())?;
    globals.set("cycle", cycle.clone())?;
    let symbols = scope.symbols.clone();
    globals.set(
        "symbol",
        Func::from(move |name: String| -> rquickjs::Result<Option<u16>> {
            symbols.resolve(&name).map_err(symbol_error_to_js)
        }),
    )?;
    let symbols = scope.symbols.clone();
    globals.set(
        "symbolInBank",
        Func::from(
            move |name: String, bank: u32| -> rquickjs::Result<Option<u16>> {
                let bank = u16::try_from(bank)
                    .map_err(|_| host_error("predicate.symbolInBank", "bank out of range"))?;
                Ok(symbols.resolve_in_bank(&name, bank))
            },
        ),
    )?;

    let gb = Object::new(ctx.clone())?;
    gb.set("regs", regs)?;
    gb.set("pc", scope.pc)?;
    gb.set("access", access)?;
    gb.set("cycle", cycle)?;
    let symbols = scope.symbols.clone();
    gb.set(
        "symbol",
        Func::from(move |name: String| -> rquickjs::Result<Option<u16>> {
            symbols.resolve(&name).map_err(symbol_error_to_js)
        }),
    )?;
    let symbols = scope.symbols;
    gb.set(
        "symbolInBank",
        Func::from(
            move |name: String, bank: u32| -> rquickjs::Result<Option<u16>> {
                let bank = u16::try_from(bank)
                    .map_err(|_| host_error("predicate.symbolInBank", "bank out of range"))?;
                Ok(symbols.resolve_in_bank(&name, bank))
            },
        ),
    )?;
    globals.set("gb", gb)?;
    Ok(restore)
}

fn parse_button(name: &str) -> rquickjs::Result<Button> {
    match name {
        "a" => Ok(Button::A),
        "b" => Ok(Button::B),
        "start" => Ok(Button::Start),
        "select" => Ok(Button::Select),
        "up" => Ok(Button::Up),
        "down" => Ok(Button::Down),
        "left" => Ok(Button::Left),
        "right" => Ok(Button::Right),
        _ => Err(host_error("gb.input", format!("unknown button {name:?}"))),
    }
}

fn symbol_error_to_js(error: SymbolResolutionError) -> JsError {
    match error {
        SymbolResolutionError::AmbiguousName { name, candidates } => host_error(
            "gb.symbol",
            format!(
                "ambiguous symbol {name:?}: {}",
                render_candidates(&candidates)
            ),
        ),
    }
}

fn render_candidates(candidates: &[SessionSymbolEntry]) -> String {
    candidates
        .iter()
        .map(|entry| match entry.bank {
            Some(bank) => format!("{bank:02X}:{:04X} {}", entry.addr, entry.name),
            None => format!("{:04X} {}", entry.addr, entry.name),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn to_js<'js, T: Serialize>(ctx: Ctx<'js>, value: T) -> rquickjs::Result<Value<'js>> {
    rquickjs_serde::to_value(ctx, value).map_err(|error| host_error("serde", error.to_string()))
}

fn js_value_to_canonical_json(value: Value<'_>) -> (JsonValue, Vec<Warning>) {
    if value.is_undefined() || value.is_function() || value.is_symbol() {
        return (
            JsonValue::Null,
            vec![Warning::new(
                "result_not_serializable",
                serde_json::json!({ "js_type": value.type_of().as_str() }),
            )],
        );
    }
    if value.is_number()
        && let Ok(number) = f64::from_js(value.ctx(), value.clone())
        && !number.is_finite()
    {
        return (
            JsonValue::Null,
            vec![Warning::new(
                "non_finite_number",
                serde_json::json!({ "value": number.to_string() }),
            )],
        );
    }
    match rquickjs_serde::from_value_strict::<JsonValue>(value) {
        Ok(value) => (canonical_json(value), Vec::new()),
        Err(error) => (
            JsonValue::Null,
            vec![Warning::new(
                "result_not_serializable",
                serde_json::json!({ "error": error.to_string() }),
            )],
        ),
    }
}

pub fn canonical_json(value: JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(values) => {
            JsonValue::Array(values.into_iter().map(canonical_json).collect())
        }
        JsonValue::Object(values) => {
            let ordered = values
                .into_iter()
                .map(|(key, value)| (key, canonical_json(value)))
                .collect::<BTreeMap<_, _>>();
            JsonValue::Object(Map::from_iter(ordered))
        }
        other => other,
    }
}

fn map_js_error(error: JsError, _started: Instant) -> ScriptError {
    if matches!(error, JsError::Allocation) {
        return ScriptError::OutOfMemory;
    }
    let message = error.to_string();
    if message.contains("stack") {
        ScriptError::StackOverflow
    } else {
        ScriptError::RuntimeException {
            message,
            line: None,
            column: None,
            function: None,
        }
    }
}

fn map_caught_or_timeout(
    error: rquickjs::CaughtError<'_>,
    started: Instant,
    timeout: Duration,
) -> ScriptError {
    if started.elapsed() >= timeout {
        return ScriptError::Timeout {
            elapsed_micros: started.elapsed().as_micros() as u64,
        };
    }
    match error {
        rquickjs::CaughtError::Exception(exception) => {
            let message = exception
                .message()
                .or_else(|| exception.stack())
                .unwrap_or_else(|| "JavaScript exception".to_owned());
            let (line, column) = parse_line_column(exception.stack().as_deref());
            if message.contains("SyntaxError") {
                ScriptError::SyntaxError {
                    message,
                    line: line.unwrap_or(0),
                    column: column.unwrap_or(0),
                }
            } else {
                ScriptError::RuntimeException {
                    message,
                    line,
                    column,
                    function: None,
                }
            }
        }
        other => ScriptError::RuntimeException {
            message: other.to_string(),
            line: None,
            column: None,
            function: None,
        },
    }
}

fn parse_line_column(stack: Option<&str>) -> (Option<u32>, Option<u32>) {
    let Some(stack) = stack else {
        return (None, None);
    };
    for token in stack.split([':', ')', '\n']) {
        if let Ok(line) = token.parse::<u32>() {
            return (Some(line), None);
        }
    }
    (None, None)
}

fn host_error(method: &'static str, message: impl Into<String>) -> JsError {
    JsError::new_from_js_message(method, "gbf-debug host binding", message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let value = serde_json::json!({"b": 1, "a": {"z": 2, "c": 3}});
        assert_eq!(
            serde_json::to_string(&canonical_json(value)).expect("json"),
            r#"{"a":{"c":3,"z":2},"b":1}"#
        );
    }

    #[test]
    fn math_random_seed_is_stable_formula() {
        let mut seed = 0xDEAD_BEEF_CAFE_BABE_u64;
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        assert_eq!(seed, 0xB6E8_5008_5308_1684);
    }
}
