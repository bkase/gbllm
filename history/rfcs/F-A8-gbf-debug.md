# RFC F-A8: `gbf-debug` — agent-facing scripted debugger CLI

| Field          | Value                                                                                                          |
|----------------|----------------------------------------------------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A8)                                                                               |
| Status         | Draft (amended 2026-05-04 to reflect F-A7 as merged)                                                           |
| Feature bead   | `bd-1o08`                                                                                                      |
| Open tasks     | T-A8.1 (`bd-1ckj`, session file format), T-A8.2 (`bd-2ulg`, rquickjs host), T-A8.3 (`bd-3psj`, gb object), T-A8.4 (`bd-3shw`, structured CLI output), T-A8.5 (`bd-7fft`, stateless CLI), T-A8.6 (`bd-2i1i`, symbol embedding), T-A8.7 (`bd-24ju`, predicate persistence), T-A8.8a (`bd-1aaz`, agent skill `SKILL.md` per agentskills.io), T-A8.8b (`bd-2j4m`, scripted runtime-ASM conformance smoke suite for emitted F-A* ROMs) |
| Closed tasks   | (none — `gbf-debug` does not yet exist as a crate at the start of F-A8)                                        |
| Plan reference | `history/planv0.md` line 158 (`gbf-debug` crate description), lines 196 + 208 (module decomposition `gbf-debug::{session, script, cli}`), lines 315–321 (`gbf-debug` design and CLI shape), lines 2716–2734 (agent debugger design rationale, JS host, deterministic execution contract, `.sym` format), line 2904 (M0 deliverable: "agent-debuggable from the first ROM that boots") |
| Dependencies   | F-A7 (`bd-3mxe`, `gbf-emu`) **— closed 2026-05-03**. F-A8 imports `Emulator::load_rom(bytes: &[u8], config: EmulatorConfig) -> Result<Self, EmuError>`, `BootMode::PostBootDmg`, `DeterminismPolicy::default()`, typed `ClockCycles` / `MCycles` / `CycleBudget`, `Snapshot { blob, lineage, trace_bank }` (derives `Serialize`/`Deserialize`) + `SnapshotLineage { rom_sha256: Hash256, boot, policy_fingerprint, emu_version, cycle_count }`, `TrapDispatcher` (with `add_pc` / `add_mem_read` / `add_mem_write` / `add_mem_rw` / `remove`), `TrapKind::{Pc{addr}, MemRead{range}, MemWrite{range}, MemRw{range}}`, `Predicate::{Always, Closure, Source}`, `TrapAction::{HaltAndReport, Continue}`, `BreakpointId`, `NormalizedTraceEvent`, and `TraceOrigin::HostBus`. F-A1 (`bd-ssm`, open) — `.sym` writer (T-A1.8 closed), ROM builder (T-A1.9 closed), `SymbolTable` / `SymEntry` types in `gbf-asm::symbols`. |
| Constitution   | `CONSTITUTION.md` §I.1 (correctness by construction), §III (shifting left), §IV (immutable runtime / reproducible builds), §V (observability and structured logs), §VI.1 (single source of truth) |

## Project context — where F-A8 sits in the plan

Like the other Epic A RFCs, this one is small in line count and large in blast radius. Every later debugging interaction the agent has with a Game Boy build flows through this CLI; every CI integration test that needs to script the emulator at finer granularity than "run to a breakpoint" runs a `gbf-debug exec` invocation; every counterfactual exploration ("what if branch X had gone the other way") materializes as a `gb.snapshot()` inside a script or a session-file copy outside one. Before any of the type definitions in §3 make sense, a reader needs to know what `gbf-debug` is *for* in the project as a whole and where it sits inside Epic A specifically. This preface answers both questions; the numbered sections that follow are the design itself.

### The whole-project framing

GBLLM is an end-to-end Rust toolchain for compiling and running small language-model inference on a stock Game Boy DMG cartridge. `planv0.md` (line 1 onward) frames the system as five cooperating product crates plus three shared contracts (`gbf-foundation`, `gbf-hw`, `gbf-abi`) plus two pieces of infrastructure (`gbf-store`, `gbf-migrate`) plus the emulator/debugger pair (`gbf-emu`, `gbf-debug`). Inside that decomposition:

- **`gbf-emu`** (F-A7, `bd-3mxe`) is the **single Rust adapter** around the gameroy emulator core. It owns the deterministic execution policy (fixed cartridge RTC where applicable, deterministic power-on RAM policy, fixed save-state metadata timestamp, host audio output disabled), breakpoint/watchpoint primitives via a registry-driven trap dispatcher, snapshot/restore via `gbf_emu::Snapshot`, framebuffer + joypad injection, and trace normalization via `NormalizedTraceEvent`.
- **`gbf-debug`** is the **agent-facing scripted debugger CLI** layered on top of `gbf-emu`. It owns nothing about emulation itself; it owns the *interface* between the agent, the emulator, and a durable session file. The interface consists of three things: (1) a stateless CLI (`init` / `exec` / `inspect`), (2) a JavaScript scripting host (rquickjs) that exposes a `gb` object backed by `gbf-emu` primitives, and (3) an on-disk session file (`.gbsess`, zstd-compressed JSON, schema-versioned) that round-trips emulator state, breakpoints/watchpoints, the embedded `SymbolTable`, a size-capped trace ring, and a one-hop `parent_sha256` lineage.

The split is deliberate. `gbf-emu` is consumed by `gbf-test` (integration suites), `gbf-bench` (cycle calibration), the eventual F-D2 harness control plane (host-side `HarnessOp` plumbing), *and* `gbf-debug`. Putting the JS host inside `gbf-emu` would force every consumer to take the rquickjs dependency; putting it on the CLI side keeps `gbf-emu` lean and lets `gbf-debug` evolve its scripting surface without disturbing the calibration pipeline. (See `planv0.md` lines 196 and 313 — `gbf-emu` "does not host a JS runtime — that lives one crate over.")

`gbf-debug` is the third bullet, and that is the entire reason this RFC exists. Without it, every agent debugging interaction would be:

- bespoke shell-out to a separate emulator binary,
- per-emulator breakpoint syntax learned from scratch,
- no durable session state across tool calls,
- symbolic-name lookups requiring the agent to parse `.sym` files manually.

With `gbf-debug` the shape collapses: one CLI surface, one scripting model (JS with a typed `gb` object), one on-disk schema (zstd-compressed `.gbsess`), one symbol-rehydration story (the entire `SymbolTable` is embedded in the session file at `init` time so a moved or deleted `.sym` never breaks the next `exec`).

### Where F-A8 sits inside Epic A

Epic A (`bd-14y`) is the **M0 foundation stack** — the first milestone in `planv0.md` §"What I would build first" (line 2901). M0's deliverable is a Game Boy that boots a deterministic, agent-debuggable ROM with a cooperative runtime skeleton, an emulator harness that talks to it, and the typed contracts every later milestone will compile against. The Epic A features and how they relate to F-A8:

| Feature | Bead | What it delivers | Relationship to F-A8 |
|---------|------|------------------|----------------------|
| F-A1 | `bd-ssm` (open; T-A1.8 + T-A1.9 closed) | `gbf-asm` typed LR35902 eDSL — `.sym` writer (`SymEntry::Display` produces `BB:AAAA name` lines), ROM builder | **Producer** of the `.sym` text and `.gb` ROM bytes that F-A8's `init` consumes. |
| F-A2 | (gbf-hw) | DMG/MBC5 target profiles + calibration schema | Sibling; F-A8 does not depend on it directly (gameroy already encodes DMG semantics). |
| F-A3 | `bd-2k2` (closed) | `gbf-abi` live execution contract types | F-A8 may *display* `BuildIdentityBlock` / `LivenessCounters` from `gbf-debug inspect` once the runtime starts emitting them, but consuming those types is an F-D2 / F-A5 concern, not an F-A8 one. F-A8 ships the surface to point at them; the actual decoding lives in the runtime nucleus. |
| F-A4 | `bd-1sv` (closed) | `BankLease`/`BankGuard` runtime ABI | Independent. F-A8 does not understand banking semantics; the `gb.read(addr, len)` primitive simply asks gameroy what is currently visible at `addr`. |
| F-A5 | `bd-2r1` | Bank0 cooperative runtime nucleus | The **first ROM that boots** under M0 is an F-A5 ROM; F-A8 is the tool the agent uses to step through that boot. F-A8 does not depend on F-A5 to ship — it depends on the emulator and on `gbf-asm`'s ROM builder, both of which exist before F-A5 lands. |
| F-A6 | `bd-3ll` (closed) | `gbf-store` content-addressed storage + `StageCache` | Independent. F-A8 does not store sessions through `gbf-store`; sessions are user-owned files at user-chosen paths. |
| F-A7 | `bd-3mxe` **(closed 2026-05-03)** | `gbf-emu` gameroy adapter, `DeterminismPolicy`, trap dispatcher, trace ring, harness slot | **Direct prerequisite, now landed.** F-A8 imports `Emulator`, `EmulatorBuilder`, `EmulatorConfig`, `BootMode`, `Snapshot { blob, lineage, trace_bank }` + `SnapshotLineage`, `CycleBudget`, `TrapDispatcher` (`add_pc` / `add_mem_*` / `remove`), `TrapKind` (parameterized by `addr` / `range`), `Predicate` (`Always` / `Closure` / `Source`), `TrapAction`, `BreakpointId`, `NormalizedTraceEvent`, `TraceOrigin::HostBus`, `EmuError`, and the `peek` / `peek_range` / `poke` / `bus_read` / `bus_write` accessors on `Emulator`. |
| **F-A8** | **`bd-1o08`** | **`gbf-debug` — this RFC.** | **The agent's debugger CLI.** |

F-A8 is **downstream of F-A7 and F-A1 inside Epic A**: its `gb` object is a Rust↔JS binding over `gbf-emu`'s `Emulator`, and its `.sym` rehydration consumes `gbf-asm`'s on-disk symbol format. F-A8 is not a hard prerequisite for F-A5 (the runtime can compile and boot without a debugger) but it is the *deliverable* that makes M0 useful: without `gbf-debug`, the first ROM that boots is opaque from the agent's perspective.

### Why this CLI has to land in M0

`planv0.md` line 2904 spells out the M0 deliverable in one phrase: "`gbf-debug` session file format and rquickjs-scripted agent CLI **so the runtime skeleton is agent-debuggable from the first ROM that boots**." That clause is the load-bearing addition. The agent's first interaction with an F-A5 ROM is supposed to be a scripted `gbf-debug exec` invocation — write a multi-step JS script, hand it off, get back structured JSON plus a new session file. If the CLI is missing, every iteration on the runtime nucleus becomes a one-off shell-out to gameroy, which means losing the symbolic-name surface, losing the durable session state, and losing the structured output that downstream tooling expects.

The plan also identifies the architectural risk that motivates *this particular shape* of CLI (lines 2722–2731). The two observations are:

1. **Agents are best at coding.** A point-and-click GUI debugger is the wrong shape; a programmable scripting host is the right one.
2. **Emulator state is cheap.** A ten-megabyte session file copies in milliseconds; counterfactual exploration is not a feature, it is a `cp`.

Both observations point at the same answer: a stateless CLI that round-trips through a session file plus a JS scripting host. That answer is what F-A8 is. (See §3.)

### The "single tool call per programmable batch" agent loop

Every `gbf-debug exec` invocation is one tool call. The agent writes a self-contained JS script, hands the script + an input session file to the CLI, and gets back:

- a JSON envelope on stdout containing `{ result, logs, session_path, session_sha256, parent_sha256, warnings, metrics? }`;
- a new session file at the requested output path, with the post-script emulator state, persisted breakpoints/watchpoints, an updated trace ring, and a one-hop `parent_sha256` pointer to the input session.

Conditional breakpoints collapse to "scripted loops with predicates": the agent writes `while (gb.regs.pc !== ENTRY) { gb.step(1); if (some_condition()) break; }`. Counterfactual exploration is `gb.snapshot(); gb.run_until(branch_a); /* observe */ gb.restore();` inside the script, or `cp parent.gbsess fork.gbsess && gbf-debug exec --in fork.gbsess ...` outside it. The JS host gives the agent the ergonomic loop-and-branch surface it would otherwise have to round-trip across many tool calls.

In short: F-A8 is the small, foundational crate that converts the planv0.md "agents are best at coding" observation into the CLI surface, and it is the prerequisite for every later agent-driven debugging interaction in the project. M0 ships when F-A1 + F-A2 + F-A3 + F-A4 + F-A5 + F-A6 + F-A7 + F-A8 all close.

## 0. TL;DR

`gbf-debug` is the **agent-facing scripted debugger CLI** layered on `gbf-emu`. It is `gbf-emu`'s sibling: where `gbf-emu` owns the gameroy adapter, deterministic execution policy, and trap primitives, `gbf-debug` owns the JS scripting host, the on-disk session file format, and the stateless `init`/`exec`/`inspect` CLI. It is the natural home for every type that crosses the agent/emulator boundary (the `gb` object, the session schema, the structured stdout envelope) and the only place we put the rquickjs dependency.

The crate ships **three modules** — `session`, `script`, `cli` — totalling roughly 1,800–2,200 LOC of production code plus ~2,000 LOC of session-schema, script-binding, and CLI integration tests. Every type that ever crosses an invocation boundary (the on-disk `Session`, the stdout `OutputEnvelope`, the stderr `ErrorEnvelope`) gets a serde round-trip test, a schema-version test, and a deterministic-output golden test. Every method on the `gb` object gets a JS-side integration test that calls it and asserts the resulting emulator state.

The seven most load-bearing decisions in this RFC are:

1. **Stateless CLI, durable session file.** No daemon, no long-lived process, no shared memory. Each invocation reads its input session, runs the script, writes its output session, exits. State that must survive across invocations is in the session file; state that lives only for the script (JS variables, transient emulator snapshots) dies when the process exits. This shape makes counterfactual exploration trivial (`cp` the session file) and makes the CLI safe to run from CI, from agents, and from human shells without any coordination.
2. **`.gbsess` is zstd-compressed JSON, schema-versioned, hard-refusal on mismatch.** No auto-migrate. A `Session` with `schema_version != CURRENT_SESSION_SCHEMA_VERSION` fails to load with a typed error and a non-zero exit code. Migration is an explicit operator action (rebuild from sources, or the day a real schema bump justifies opening a follow-up bead). Why JSON: human-readable when uncompressed (debugging the debugger), trivial to round-trip in Rust + serde, and zstd compression neutralizes the size cost. Why hard refusal: the workspace ships zero versioned session schemas in production today; the cost of "rebuild from sources" is essentially zero, while the cost of carrying a migration scaffold per F-A6's argument is paid every day.
3. **The `SymbolTable` is embedded in the session, not referenced by path.** When `gbf-debug init` ingests a `.sym` file, it parses the file (using `gbf-asm::symbols::parse_sym_entries`), constructs an in-session `SessionSymbolTable`, and serializes the whole thing into the session bytes. Subsequent `exec` invocations rehydrate from the embedded copy. This means moving, deleting, or rebuilding the `.sym` between invocations never breaks the next session load. Cost: a few KB to ~100 KB of session size (well under any reasonable size budget once compressed).
4. **The JS host is rquickjs with an interrupt-handler-driven wall-clock timeout.** rquickjs (Rust bindings to QuickJS-NG) gives us an embeddable JavaScript engine with no async runtime, a small binary footprint, and a typed `class!` macro for binding Rust types to JS prototypes. Wall-clock timeout is implemented via the QuickJS interrupt handler hook: every N opcodes the host checks elapsed time against the configured `script_timeout` (default 30 s) and aborts with `ScriptError::Timeout`. `Date.now()` and `Math.random()` are deterministically stubbed so a script's behavior cannot depend on wall-clock time or system entropy.
5. **`gb.snapshot()`/`gb.restore()` is in-script, transient; on-disk session lineage is one-hop `parent_sha256`.** The two snapshot mechanisms are deliberately separate. In-script snapshots use `gbf-emu::Emulator::snapshot` to produce an opaque blob held in the JS heap; calling `restore()` brings the emulator back to that point and the snapshot is dropped at script end. Cross-invocation lineage uses the session file: each `exec` writes a new session whose `parent_sha256` field points at the SHA-256 of the input session bytes. The agent's counterfactual exploration vocabulary is `snapshot/restore` for "try this branch, undo it, try the other" and `cp` for "fork this entire session and explore independently."
6. **Structured output is mandatory, and includes argument-parse failures.** `gbf-debug exec` writes exactly one JSON object to stdout: `{ result, logs, session_path, session_sha256, parent_sha256, warnings, metrics? }`. `result` is the value of the script's `globalThis.result` (or JSON `null`); JS values that don't serialize cleanly (`undefined`, functions, Symbols, `NaN`/`Infinity`, `BigInt`, cycles) become JSON `null` plus a `Warning`, and every JS-produced JSON object is recursively canonicalized (object keys sorted lexicographically) so envelope output is byte-stable regardless of `serde_json` feature unification. `logs` is an ordered array of `{ message, data, ts_micros_since_script_start }` records produced by `log(msg, data)` calls; timestamps come from the deterministic virtual clock, not host time. Errors — including clap argument-parse failures — go to stderr as a JSON `ErrorEnvelope`. There is no human-prose CLI output. (The `inspect` command is the same shape with the script-execution fields replaced by a state dump; see §3.5.4.)
7. **Predicates have two explicit kinds: closure-shaped (invocation-local) and stringified-source (persisted).** `gb.add_breakpoint(addr)` with no predicate is a plain unconditional breakpoint and persists. `gb.add_breakpoint(addr, "regs.a == 0x42")` is a *stringified-source* predicate that is persisted verbatim and re-parsed in a read-only `{regs, trap, symbol}` scope on the next `exec`. `gb.add_breakpoint(addr, () => regs.a == 0x42)` is a *closure-shaped* predicate that lives only for the current invocation and is **not** written to the session; persisting a conditional closure as an unconditional breakpoint would silently change semantics ("break when A == 0x42" → "break always") which is worse than dropping the breakpoint outright. The agent must decide explicitly which kind to use; the API never tries to capture and serialize a closure's environment.

The new surface adds roughly 1,800–2,200 LOC of production code plus about 2 KLOC of integration tests, golden session fixtures, and CLI smoke tests. The *public* surface is intentionally small: three CLI subcommands, one binary, one library facade (`gbf_debug::Session::load`, `gbf_debug::run_script`) for tests that want to drive the host without going through the binary. There is no new emulator dependency; gameroy comes through F-A7. There is no new symbol format; `.sym` parsing comes through F-A1.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A new crate `gbf-debug` with three modules (`session`, `script`, `cli`) plus a binary target `gbf-debug` that ships the CLI.
- An on-disk **session file format** (`.gbsess`):
  - zstd-compressed JSON wire format with a fixed magic header (`"GBSE"`) preceding the zstd frame so the format is identifiable in a hex dump;
  - explicit `SCHEMA_VERSION: u32 = 1` constant; hard refusal on mismatch with `SessionLoadError::SchemaMismatch { observed, current }`;
  - typed `Session` struct embedding: `schema_version`, `parent_sha256` (`Option<[u8; 32]>`), `rom_sha256` (`[u8; 32]`), `rom` (`RomBlob`, base64 of original `.gb` bytes), `emulator_snapshot` (`EmulatorSnapshotBlob` — a transparent newtype over `gbf_emu::Snapshot { blob, lineage, trace_bank }`, which already derives `Serialize`/`Deserialize`; the inner `blob: Vec<u8>` becomes base64 at the JSON layer, the structured `lineage` and `trace_bank` stay structured), `symbols` (`SessionSymbolTable`), `breakpoints` (`Vec<BreakpointPersisted>`), `watchpoints` (`Vec<WatchpointPersisted>`), `trace_ring` (`TraceRing`), and a small `metadata` block for build identity (`abi_version_observed`, `created_at_micros_since_init`, `notes`).
- A typed **rquickjs scripting host**:
  - `ScriptHost::new(script_source, timeout)` builds a fresh QuickJS context per invocation; the context is dropped at end of `exec`;
  - `ScriptHost::run(&mut self, gb_binding) -> Result<ScriptOutcome, ScriptError>` evaluates the script with the `gb` object bound;
  - the QuickJS interrupt handler is wired to a wall-clock-deadline checker (default `Duration::from_secs(30)`);
  - `Date.now()` and `Math.random()` are deterministically stubbed (see §3.2.4);
  - error messages preserve the script's source position (`script_line`, `script_column`, `script_function`);
  - `console.log`/`print` are *not* exposed; the only structured output path is `log(msg, data)`.
- A typed **`gb` object binding** exposing the surface from planv0.md line 318:
  - `gb.regs` — read-only snapshot object with fields `pc`, `sp`, `a`, `b`, `c`, `d`, `e`, `h`, `l`, `f`, `bc`, `de`, `hl`, `ime`; `ime` is `"disabled" | "enabled" | "to_be_enable"` and mirrors F-A7's tri-state `ImeSnapshot`;
  - `gb.read(addr, len) -> Uint8Array`, `gb.write(addr, bytes)` — side-effect-free debugger access backed by F-A7 `peek_range` / `poke`; rejects IO and other unsupported raw regions with a typed host-binding error;
  - `gb.bus_read(addr, len) -> Uint8Array`, `gb.bus_write(addr, bytes)` — side-effecting adapter-synthesized CPU-bus operations backed by F-A7 `bus_read` / `bus_write`; may advance emulator state and emit `TraceOrigin::HostBus` events;
  - `gb.step(n)`, `gb.run_until(pc, max_m_cycles?)`, `gb.run_until_breakpoint(max_m_cycles?)`;
  - `gb.add_breakpoint(addr, predicate?)`, `gb.remove_breakpoint(addr)`, `gb.list_breakpoints()`;
  - `gb.add_watchpoint(addr, kind, predicate?)`, `gb.remove_watchpoint(addr, kind)`, `gb.list_watchpoints()`;
  - `gb.snapshot() -> SnapshotHandle`, `gb.restore(handle)` — in-script transient branching;
  - `gb.symbol(name) -> Option<u16>`, `gb.symbol_at(addr) -> Option<string>`;
  - `gb.framebuffer() -> Uint8Array`, `gb.input(buttons)`;
  - `gb.trace_ring() -> Array<TraceEventJs>`, `gb.clear_trace()`;
  - `log(msg, data?)` — appends a structured log entry;
  - `result = expr` — the script's final value of the global `result` variable becomes the `result` field of the output envelope.
- A **stateless CLI** with three subcommands implemented via `clap`:
  - `gbf-debug init --rom <path> [--sym <path>] --out <session.gbsess>` — load the ROM, parse the `.sym` if provided (via `gbf-asm::symbols::parse_sym_entries`), forge a fresh `Session` with PC at `$0100` (the post-bootrom entrypoint) and an empty trace ring / breakpoint set / watchpoint set, write the zstd-compressed JSON to the output path;
  - `gbf-debug exec --in <session.gbsess> --script <path> --out <session.gbsess> [--timeout <seconds>]` — load the session, hydrate the emulator, run the script, snapshot the emulator state, write the output session, emit the JSON envelope to stdout;
  - `gbf-debug inspect <session.gbsess>` — load the session, *do not* run any script, emit a JSON dump of the session header, register state, breakpoints, watchpoints, trace ring, and symbol-table summary to stdout.
- **Symbol embedding & rehydration**: `init` parses `.sym` lines via `gbf-asm::symbols::parse_sym_entries` and constructs a `SessionSymbolTable` (a deterministically-ordered name↔addr index) embedded in the session. `exec` rehydrates the table from the session bytes (never from a sidecar `.sym` path).
- **Predicate persistence**: `BreakpointPersisted::predicate` is a typed enum `{ None, StringifiedSource(String) }`. The closure-shaped variant exists only as a JS-side runtime value; it is *not* a `Persisted` field.
- **Determinism**: every successful invocation that does not hit a wall-clock watchdog produces the same output session bytes and the same default stdout envelope for the same semantic inputs:
  `(input session bytes, script bytes, output path string as emitted in the envelope, CLI arguments that affect semantics, gbf-debug binary version)`.
  Host-duration metrics are not emitted in the default envelope (they are opt-in via `--emit-metrics` and excluded from determinism golden tests).
  Wall-clock watchdog failures are liveness failures, not deterministic semantic outcomes; by default they do not write an output session. A separate opt-in flag may write a partial session marked `determinism: "nondeterministic_partial"`.
- **Layered tests**: serde round-trip, schema-version negative tests, JS host integration tests for every `gb.*` method, golden session fixtures for `init` and `inspect`, and a small end-to-end "tiny ROM" test that runs `init → exec → inspect` against a checked-in `.gb` produced by `gbf-asm::rom`.
- **Agent skill**: an Agent Skill at `.agents/skills/gbf-debug-usage/SKILL.md` per the agentskills.io specification (frontmatter `name` / `description` / optional `compatibility` and `metadata`, body under 500 lines per progressive-disclosure guidance, longer worked examples in `references/` and reusable JS recipe templates in `assets/`). The skill teaches an agent how to drive F-A8 correctly: when to use `init` vs `exec` vs `inspect`; the JSON-envelope I/O contract on stdout/stderr; the closure-vs-stringified predicate split; the side-effect-free `read`/`write` vs side-effecting `bus_read`/`bus_write` split; the deterministic virtual-clock contract for `Date.now()` and `log()` timestamps; the one-shot tool-call agent loop (write script → `exec` → read envelope → repeat); `gb.snapshot()`/`gb.restore()` for in-script branching vs `cp session.gbsess` for cross-invocation forks; how to set `globalThis.result`; the symbol-ambiguity story (`gb.symbol` vs `gb.symbol_in_bank`); and the `run_until*` budget contract (`default_run_budget`, `max_step_instructions_per_call`, `max_m_cycles`). See §3.7.

### 1.2 Non-goals (deferred)

- **The gameroy emulator core itself.** F-A7 (`bd-3mxe`) ships the gameroy adapter. F-A8 imports `gbf-emu::Emulator` and never re-implements an emulator backend.
- **`DeterminismPolicy` implementation.** F-A7 (T-A7.3, `bd-10y1`) owns it. F-A8 *constructs* a `DeterminismPolicy` for each invocation but does not define one.
- **Trap dispatcher implementation.** F-A7 (T-A7.4, `bd-19as`) owns the registry-driven dispatcher. F-A8's `gb.add_breakpoint`/`gb.add_watchpoint` calls into the dispatcher API.
- **Trace normalization.** F-A7 (T-A7.5, `bd-14yy`) owns the canonical `NormalizedTraceEvent` shape. F-A8 stores normalized events in `TraceRing` and exposes them to JS as `gb.trace_ring()`; it does not re-normalize.
- **The harness command/result block (F-D2 control plane).** `HarnessOp::StepSlice` / `RunUntilCheckpoint` / etc. live in `gbf-abi::harness` (F-A3) and the host-side polling lives in F-A7 T-A7.6 + F-D2. F-A8 *may* eventually expose `gb.harness.send(op)` but that is a follow-up bead; in M0 the agent uses raw `gb.step` / `gb.run_until` against PC values.
- **The `BuildIdentityBlock` / `LivenessCounters` / `FaultCode` decode surface.** F-A3 owns those types; F-A8 may surface their *bytes* via `gb.read(addr, len)` in M0. A future `gb.identity()` / `gb.liveness()` accessor that decodes into typed objects is a follow-up bead, gated on the runtime emitting them at known offsets (F-A5).
- **CGB / GBC features.** DMG only.
- **A REPL.** The CLI is one-shot per invocation. No interactive prompt, no readline, no live JS console. The agent's loop is `write_script → exec → read_envelope → repeat`, not `interact_at_a_prompt`.
- **Networked or multi-process operation.** No daemon, no IPC, no shared memory, no tokio / async-std. All operations are synchronous `std::io` and synchronous QuickJS.
- **Persistent JS context across invocations.** Each `exec` builds a fresh QuickJS context. Anything the script wants to remember it must `result = ...` out, or write into the session via `gb.add_breakpoint(..., "...")`. Module-level JS state does not survive.
- **`gbf-store` integration.** Sessions are user-owned files at user-chosen paths. They are not pinsetted, not GC'd, not archived through `gbf-store`. (A future bead may add `gbf-debug archive --pinset <name>` for batch experiments; that is not F-A8.)
- **Multi-cartridge multi-emulator dispatching.** One ROM, one emulator, one session per invocation.
- **A graphical front-end.** No SDL, no eframe, no imgui. Framebuffer access is `gb.framebuffer() -> Uint8Array`; if the agent wants to render it, the agent writes a script to dump the bytes.
- **Auto-migration of old sessions.** `SessionLoadError::SchemaMismatch` is fatal. Per the F-A6 deferral argument: the cost of "rebuild from sources" is essentially zero today, and a migration scaffold pays a daily maintenance tax for hypothetical future bumps. The first real session-schema bump opens a follow-up bead with explicit migrators.
- **`unsafe` outside the rquickjs FFI itself.** `#![forbid(unsafe_code)]` is set on every `.rs` file in the `gbf-debug` crate. rquickjs's internal FFI is `unsafe` but is encapsulated in the upstream crate; we do not add any new `unsafe` lines.

## 2. Background and existing state

### 2.1 What is already in tree

- **`gbf-asm::symbols`** (T-A1.8 closed): `SymbolTable`, `SymbolName`, `SymbolAddress`, `SymbolSegment`, `SymError`, `SymEntry`, `parse_sym_entries(input: &str) -> Result<Vec<SymEntry>, SymError>`, and `write_sym(layout, symbols, opts) -> Result<String, SymError>`. The `.sym` line format is `BB:AAAA name` (banked) or `AAAA name` (unbanked); `SymEntry::Display` produces those lines and `SymEntry::FromStr` parses them. F-A8 consumes `parse_sym_entries` directly and converts each `SymEntry` into a `SessionSymbolEntry` (see §3.1.4).
- **`gbf-asm::rom`** (T-A1.9 closed): the ROM builder that produces a valid `.gb` byte stream from a `LayoutPlan` + section bytes. F-A8 does not build ROMs; it consumes them as opaque byte strings to feed `Emulator::load_rom`.
- **`gbf-emu`** (F-A7 closed 2026-05-03): the crate ships with the module layout `adapter`, `determinism`, `harness`, `primitives`, `trace_ring`, `trap` (the original sketches `adapters.rs` / `breakpoints.rs` / `trace.rs` were replaced during F-A7 implementation and no longer exist). Public re-exports from `gbf-emu/src/lib.rs` (the names F-A8 imports verbatim):
  - From `adapter`: `BootMode`, `BootRomImage`, `Emulator`, `EmulatorBuilder`, `EmulatorConfig`.
  - From `determinism`: `AudioOutputMode`, `CartridgeRtcMode`, `DeterminismPolicy`, `DeterminismPolicyBuilder`, `FIXED_CARTRIDGE_RTC_UNIX_MS`, `FIXED_SAVE_STATE_UNIX_MS`, `PowerOnRamPolicy`, `SaveStateMetadataMode`.
  - From `harness`: `HarnessChannel`, `HarnessCommand`, `HarnessResult`, `HarnessSlot`.
  - From `primitives`: `BootModeLineage`, `ClockCycles`, `Color`, `CpuIdleState`, `CycleBudget`, `DMG_FRAME_CLOCK_CYCLES`, `EmuError`, `EmuVersionTag`, `Flags`, `Framebuffer`, `GitSha`, `ImeSnapshot` (`Disabled` / `Enabled` / `ToBeEnable`), `JoypadFrame`, `MCycles`, `Regs`, `RunOutcome` (`BudgetElapsed { observed, requested }` / `TrapHit { trap_id, kind, observed }` / `Idle { state, observed }`), `Snapshot { blob, lineage, trace_bank }` (derives `Serialize`/`Deserialize`), `SnapshotLineage { rom_sha256: Hash256, boot, policy_fingerprint, emu_version, cycle_count }`, `StepOutcome`, `TrapPredicateError`.
  - From `trace_ring`: `BankSnapshot`, `BankSwitchSource`, `NormalizedTraceEvent`, `TraceCursor`, `TraceDropPolicy`, `TraceMapper`, `TraceOrigin` (`GuestCpu` / `Dma` / `HostBus` / `HostPoke`).
  - From `trap`: `AddressRange`, `AddressRangeError`, `BreakpointId`, `EmuReadOnlyMemory`, `EmuReadOnlyView`, `MemoryAccess`, `MemoryAccessKind`, `Predicate` (`Always` / `Closure(Box<TrapPredicate>)` / `Source(String)`), `PredicateSpec` (`Always` / `Source(String)` — the persistence-friendly subset), `RemovedTrap`, `TrapAction` (`HaltAndReport` / `Continue`), `TrapContext` (fields: `regs`, `pc`, `access`, `cycle`, `view: EmuReadOnlyView`), `TrapDispatcher` (with `add_pc(addr, predicate, action)` / `add_mem_read(range, predicate, action)` / `add_mem_write(...)` / `add_mem_rw(...)` / `remove(id)` / `list()` / `export_persistable_specs()` / `clear()`), `TrapKind::{Pc { addr }, MemRead { range }, MemWrite { range }, MemRw { range }}`, `TrapListEntry`, `TrapPersistenceError`, `TrapSpec`.

  F-A8 also consumes F-A7's split memory accessors on `Emulator`: side-effect-free `peek(addr) -> Result<u8, EmuError>` / `peek_range(start: u16, len: usize) -> Result<Vec<u8>, EmuError>` / `poke(addr, value) -> Result<(), EmuError>` (debugger reads/writes that do not advance state and do not emit guest trace events) and side-effecting `bus_read(addr) -> Result<u8, EmuError>` / `bus_write(addr, value) -> Result<(), EmuError>` (adapter-synthesized CPU-bus operations that record `TraceOrigin::HostBus`).

  This RFC was originally drafted before F-A7 landed and gated implementation on a post-merge amendment. **F-A7 has now merged**; the surface above is taken from `gbf-emu`'s public exports as of `c269c4f` (Add F-A7 RFC and align gbf-emu beads with API-surface findings) and the §4.3 adapter shape table is authoritative. Where the original RFC named symbols that did not survive implementation (`EmulatorError`, `register_pc`/`register_mem`/`unregister`, `MemReadWrite`, `step(n) -> StepResult`, `RunOutcome::TimeBudgetExpired`, `JoypadState`, `RegsSnapshot` as the `Emulator::regs` return type, `Snapshot::to_bytes`/`from_bytes`), the F-A8 implementation uses the actually-landed names.
- **`gbf-foundation`** (in tree, multiple closed beads): `Hash256`, `SemVer`. F-A8 uses `Hash256` for `parent_sha256` and `rom_sha256` (or, if `Hash256` is not yet a stable transparent newtype over `[u8; 32]`, F-A8 stores raw `[u8; 32]` and converts at the boundary).

### 2.2 What does *not* yet exist

- The `gbf-debug` crate itself. There is no `gbf-debug/` directory, no `Cargo.toml` entry under `[workspace] members = [...]`, no source files. F-A8 creates the crate from scratch.
- All concrete types in §3.1 (session schema), §3.2 (script host), §3.3 (gb binding), §3.4 (CLI), §3.5 (output envelopes).
- The rquickjs dependency. F-A8 adds `rquickjs` to `gbf-debug/Cargo.toml`; no other crate in the workspace depends on it and none should without an explicit RFC.
- The `zstd` dependency. F-A8 adds it. (Used by the session writer / reader only.)
- The `clap` dependency. F-A8 adds it for the binary target only.
- Any test fixture session files (`tests/fixtures/*.gbsess`).

### 2.3 Downstream pressure on this design

`gbf-debug` is initially consumed by exactly one party: **the agent**. There are no stable Rust-code consumers in F-A8. Specifically:

- **`gbf-test`** (Epic A and beyond) will eventually run `gbf-debug exec` as a subprocess from integration tests where a scripted boot is the cleanest way to express "boot the runtime, run until checkpoint X, assert state Y." But `gbf-test` is not blocked on F-A8 closing — it can also drive the emulator directly through `gbf-emu` for its own tests. The CLI-as-tool consumption is opportunistic, not load-bearing.
- **`gbf-bench`** (Epic E) does not consume `gbf-debug`. Calibration runs are bulk and need direct emulator access without a JS host in the loop.
- **`gbf-cli`** (the top-level user CLI) may eventually re-export `gbf-debug` subcommands as `gbf debug init / exec / inspect`, but that's a thin wrapping decision left to whichever bead first wires up `gbf-cli`. F-A8 ships a standalone `gbf-debug` binary; the wrapping is additive.
- **No oracle, runtime, or compiler crate consumes `gbf-debug`.** The dependency graph stays one-way: `gbf-debug → gbf-emu → gameroy`, `gbf-debug → gbf-asm`, `gbf-debug → gbf-foundation`. Nothing depends on `gbf-debug` as a library.

This makes F-A8 unusually low-risk in terms of downstream blast radius. A bug in the session schema does not cause a runtime memory corruption; it causes the next `exec` to fail with a typed error. A bug in the JS binding does not cause a compiler miscompile; it causes the agent to write a follow-up bead.

### 2.4 Plan and constitution grounding

This RFC threads several plan rules tightly:

- **planv0.md line 158**: `gbf-debug` is the agent-facing scripted debugger CLI with stateless session files, rquickjs scripting host, and a programmable machine-interface. → §3 ships exactly that decomposition.
- **planv0.md line 196**: `gbf-emu` "does not host a JS runtime — that lives one crate over." → rquickjs is a `gbf-debug` dependency, never a `gbf-emu` dependency. The F-A7 RFC and F-A8 RFC both explicitly carry this rule.
- **planv0.md lines 315–321**: the canonical ownership statement for `gbf-debug`: session file, scripting host, stateless CLI, predicate-persistence policy. → §3.1, §3.2, §3.3, §3.4 mirror those bullets one-to-one.
- **planv0.md lines 2722–2731**: the agent debugger design rationale (agents are best at coding; emulator state is cheap; scripted loops with predicates; counterfactual exploration via `snapshot()`/`restore()` or session-file copy). → the entire shape of §3 is shaped by these observations.
- **planv0.md line 2732**: "Determinism is a `gbf-emu` policy, not a per-consumer choice." → F-A8 *constructs* a `DeterminismPolicy::default()` and passes it to the emulator; it does not invent its own determinism semantics. The two JS-side stubs (`Date.now`, `Math.random`) are *additional* determinism guards on the JS host that have no analog in the emulator.
- **planv0.md line 2734**: the `.sym` format is `BB:AAAA name` per line and is the format the embedded `SymbolTable` is hydrated from. → §3.1.4 and §3.6 explicitly route through `gbf-asm::symbols::parse_sym_entries`.
- **planv0.md line 2904**: M0 deliverable phrasing — "agent-debuggable from the first ROM that boots." → §1.1 goals enumerate every accessor the agent needs to debug an F-A5 boot script.

Constitutional grounding:

- **§I.1 (correctness by construction)** — the session schema uses typed enums for predicate kinds, watchpoint kinds, and trace-event categories; `SessionLoadError` is exhaustive; `BreakpointPersisted::predicate` cannot accidentally drift into a half-typed state.
- **§III (shifting left)** — schema mismatch is caught at session-load time, not mid-script; JS host errors carry source position so the agent does not have to re-derive failure location from a stack trace.
- **§IV.3 (reproducible builds)** — every successful `exec` invocation that does not hit the wall-clock watchdog is a pure function of its semantic inputs (§4.1); the determinism golden test pins the SHA-256 of a checked-in fixture session after one `init` and one `exec` round.
- **§V.1 (structured logs)** — there is one and only one CLI output shape per command, and it is JSON. There is no mode where `gbf-debug` emits human-prose status messages to stdout.
- **§V.3 (silence on success, loud on failure)** — `init`/`exec`/`inspect` exit `0` with the JSON envelope on success; on failure they exit non-zero with a JSON `ErrorEnvelope` to stderr.
- **§VI.1 (single source of truth)** — the `.sym` format is owned by `gbf-asm::symbols`; `gbf-debug` re-uses `parse_sym_entries` rather than implementing its own parser. The `NormalizedTraceEvent` shape is owned by `gbf-emu`; `gbf-debug` re-uses it rather than re-normalizing.

## 3. Module-by-module design

This section walks every module that ships in F-A8. Each subsection corresponds to one or more child tasks: §3.1 → T-A8.1 + T-A8.6 + T-A8.7, §3.2 → T-A8.2, §3.3 → T-A8.3, §3.4 → T-A8.5, §3.5 → T-A8.4, §3.7 → T-A8.8a. T-A8.8b (`bd-2j4m`, the runtime-ASM conformance smoke suite) is a follow-up bead and is not represented by a §3 module; see §11.9.

### 3.1 `session.rs` — `Session`, `BreakpointPersisted`, `WatchpointPersisted`, `TraceRing`, schema versioning (T-A8.1, parts of T-A8.6, T-A8.7)

#### 3.1.1 The `Session` struct

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub schema_version: u32,
    pub parent_sha256:  Option<[u8; 32]>,
    pub rom_sha256:     [u8; 32],
    pub rom:            RomBlob,                  // base64 of original `.gb` bytes (see §3.1.6)
    pub emulator_snapshot: EmulatorSnapshotBlob,  // newtype over `gbf_emu::Snapshot`; serde-derived (see §3.1.6)
    pub symbols:    SessionSymbolTable,           // §3.1.4
    pub breakpoints: Vec<BreakpointPersisted>,
    pub watchpoints: Vec<WatchpointPersisted>,
    pub trace_ring: TraceRing,
    pub metadata:   SessionMetadata,
}

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub abi_version_observed: Option<gbf_abi::AbiVersion>,
    pub created_at_micros_since_init: u64,    // monotone since `init` time, NOT wall-clock
    pub notes: BTreeMap<String, String>,      // free-form, ordered
}
```

The two on-disk identity fields are:

- `rom_sha256` — the SHA-256 of the original `.gb` ROM bytes. This is the *cartridge identity*; it never changes across the lifetime of a session lineage. F-A8 stores the ROM bytes explicitly in `Session::rom` and stores the F-A7 `Snapshot` separately in `emulator_snapshot`. Subsequent `exec` invocations construct an `Emulator` with `EmulatorConfig { boot_mode: BootMode::PostBootDmg, policy: DeterminismPolicy::default(), .. }`, load the embedded ROM bytes, and restore `session.emulator_snapshot.0`. F-A8 does not assume that the gameroy save-state format embeds cartridge ROM bytes; the explicit ROM blob is the load-bearing contract for the self-contained `.gbsess` story. `Session::load` rejects any session whose `sha256(session.rom.0) != session.rom_sha256` (`SessionLoadError::RomHashMismatch`) or whose `emulator_snapshot.lineage.rom_sha256 != session.rom_sha256` (`SessionLoadError::SnapshotRomMismatch`).
- `parent_sha256` — the SHA-256 of the *input session bytes* for the current `exec` invocation, or `None` for sessions produced by `init`. One-hop only; F-A8 does not maintain a chain.

`abi_version_observed` is `None` until the runtime starts emitting a `BuildIdentityBlock` (F-A5 + F-A3 territory). The field exists in the schema from day one so adding the decode in a follow-up bead does not bump `SCHEMA_VERSION`.

#### 3.1.2 On-disk container format

The file format is:

```
+----------------+----------------+--------------------------+
| MAGIC (4 B)    | FLAGS (4 B)    | ZSTD frame (compressed   |
| "GBSE"         | u32 little-end | UTF-8 JSON of `Session`) |
+----------------+----------------+--------------------------+
```

`MAGIC = b"GBSE"`. `FLAGS` is reserved for future use; F-A8 sets it to `0x00000000` and `Session::load` rejects any non-zero `FLAGS`. The JSON is canonical UTF-8 (no BOM); `BTreeMap` is used wherever ordering matters so the JSON serialization is deterministic. The zstd compression level is fixed at `3` (a balance between speed and ratio that is widely-used and that we pin to make the output bytes deterministic).

```rust
impl Session {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SessionLoadError>;
    pub fn load_bytes(bytes: &[u8]) -> Result<Self, SessionLoadError>;
    pub fn write(&self, path: impl AsRef<Path>) -> Result<[u8; 32], SessionWriteError>;
    pub fn to_bytes(&self) -> Result<Vec<u8>, SessionWriteError>;
    pub fn sha256(&self) -> Result<[u8; 32], SessionWriteError>; // sha256(to_bytes())
}
```

`load` and `load_bytes` validate the magic, the FLAGS, the zstd frame, and finally the schema version. `write` computes the SHA-256 of the serialized bytes, fsyncs, and renames atomically (`tmp_<random>` → final path).

#### 3.1.3 Schema versioning

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLoadError {
    BadMagic { observed: [u8; 4], expected: [u8; 4] },
    BadFlags { observed: u32 },
    Truncated { observed: usize, minimum: usize },
    ZstdDecode(String),
    JsonDecode(String),
    SchemaMismatch { observed: u32, current: u32 },
    RomHashMismatch { observed: [u8; 32], expected: [u8; 32] },
    SnapshotRomMismatch { snapshot_rom_sha256: [u8; 32], session_rom_sha256: [u8; 32] },
    UnsupportedBootMode { observed: String },     // F-A8 M0 sessions are PostBootDmg-only
}
```

`SchemaMismatch` is fatal. There is no `Session::migrate`, no `SCHEMA_VERSION_MIN`, no "tolerate previous-minor schema." If the `.gbsess` was written by a previous schema, the operator regenerates it by re-running `init`. Per the F-A6 deferral argument, the cost of "rebuild from sources" is essentially zero in M0; the cost of carrying a migration scaffold is paid every day. The first real session-schema bump opens a follow-up bead.

#### 3.1.4 `SessionSymbolTable` (T-A8.6)

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSymbolTable {
    pub entries: Vec<SessionSymbolEntry>,    // sorted by (bank, addr, name) for determinism
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionSymbolEntry {
    pub bank: Option<u16>,    // None for unbanked (HRAM/WRAM) symbols
    pub addr: u16,
    pub name: String,
}

impl SessionSymbolTable {
    pub fn from_sym_text(input: &str) -> Result<SymbolHydration, SymbolHydrationError>;
    pub fn resolve(&self, name: &str) -> Result<Option<u16>, SymbolResolutionError>;
    pub fn resolve_in_bank(&self, name: &str, bank: u16) -> Option<u16>;
    pub fn resolve_at(&self, addr: u16) -> Result<Option<&str>, SymbolResolutionError>;
    pub fn resolve_at_in_bank(&self, addr: u16, bank: u16) -> Option<&str>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolHydration {
    pub table: SessionSymbolTable,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolResolutionError {
    AmbiguousName { name: String, candidates: Vec<SessionSymbolEntry> },
}
```

`from_sym_text` calls `gbf_asm::symbols::parse_sym_entries(input)`, converts every `SymEntry` to a `SessionSymbolEntry`, and sorts the result by `(bank, addr, name)`. The sort is the determinism guard: the `gbf-asm` writer already produces deterministic output, but rehydration via `parse_sym_entries` returns a `Vec<SymEntry>` in input order, and we want the in-session table to be canonical regardless of the input file's line order.

`SymbolHydrationError` wraps `gbf_asm::symbols::SymError` for genuinely-malformed `.sym` lines (invalid hex, missing name, etc.). **Duplicate names are not fatal** — they are reported as `Warning { kind: "duplicate_symbol_name", ... }` entries inside the returned `SymbolHydration`, and every entry (including the duplicate) is preserved in the table. Banked ROMs routinely emit duplicate unqualified names because the same source label can land in multiple banks.

`resolve(name)` does not silently pick a winner. If a name resolves to exactly one entry, `Ok(Some(addr))`. If it resolves to none, `Ok(None)`. If multiple banked entries share the same name, `Err(SymbolResolutionError::AmbiguousName { name, candidates })`; the JS-side surface is `gb.symbol(name)` raising `HostBindingError::AmbiguousSymbol`, and the script must call `gb.symbol_in_bank(name, bank)` to disambiguate. The `bank` field is preserved in the entry so `resolve_in_bank` is well-typed.

#### 3.1.5 `BreakpointPersisted` and `WatchpointPersisted` (T-A8.7)

```rust
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum WatchpointKind {
    Read,
    Write,
    ReadWrite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistedPredicate {
    None,
    StringifiedSource(String),    // a JS expression string; re-parsed every exec
}
```

The two-variant enum is deliberate.

- A breakpoint set with `gb.add_breakpoint(addr)` (no predicate) persists as `BreakpointPersisted { addr, predicate: PersistedPredicate::None, enabled: true }`. It fires unconditionally on every visit to `addr`.
- A breakpoint set with `gb.add_breakpoint(addr, "regs.a == 0x42")` (string predicate) persists as `BreakpointPersisted { addr, predicate: PersistedPredicate::StringifiedSource("regs.a == 0x42"), enabled: true }`. The next `exec` re-parses the string and rebuilds the predicate.
- A breakpoint set with `gb.add_breakpoint(addr, () => regs.a == 0x42)` (closure predicate) is **invocation-local** and is *not* written to the session. The output envelope includes `Warning { kind: "predicate_not_persisted", ... }` so the agent learns that the closure did not survive. The CLI never converts a conditional closure breakpoint into an unconditional persisted breakpoint — that would silently change semantics ("break when A == 0x42" → "break always") on the next invocation, which is worse than dropping the breakpoint outright.

String predicates are evaluated in a restricted read-only environment mirroring F-A7's `TrapContext`: `{ regs, pc, access, cycle, symbol, symbolInBank }`. They cannot call mutating debugger methods such as `gb.write`, `gb.step`, `gb.restore`, or `gb.add_breakpoint` — those names simply do not exist in the predicate scope. This avoids reentrancy hazards while the trap dispatcher is already handling a trap and keeps the predicate scope tight enough that the agent's intent is unambiguous from the source string alone.

For memory watchpoints, `regs` and `pc` are post-instruction under F-A7's M0 `io_trace` backend. The `access` object carries the matched address, value, and access kind; predicates that need exact pre-access state must instead use PC traps near the access site.

The `enabled` field defaults to `true`; the JS-side `gb.disable_breakpoint(addr)` (a follow-up convenience method, not in M0) would flip it. F-A8 ships the field but does not ship the disable method; the field exists so adding it is non-breaking.

`WatchpointKind::ReadWrite` is the union of `Read` and `Write`. An incoming memory access fires the watchpoint if its kind is `Read` and the watchpoint kind is `Read` or `ReadWrite`; analogously for `Write`. F-A7's `TrapKind::MemRw { range }` covers the same case at the trap-dispatcher layer (F-A7 names it `MemRw`, not `MemReadWrite`); F-A8's `WatchpointKind` is the JS-side enum, kept distinct from F-A7's parameterized `TrapKind` variants so the JS surface does not have to construct `AddressRange` values or import F-A7 types directly. F-A8 maps `WatchpointKind::Read` → `TrapDispatcher::add_mem_read(AddressRange::new(addr, addr)?, predicate, action)`, `Write` → `add_mem_write`, `ReadWrite` → `add_mem_rw`.

#### 3.1.6 `RomBlob` and `EmulatorSnapshotBlob` (cartridge bytes + F-A7 snapshot)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RomBlob(pub Vec<u8>);

// Serialized as a base64 string; deserialized from a base64 string.

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmulatorSnapshotBlob(pub gbf_emu::Snapshot);

// `gbf_emu::Snapshot` derives `Serialize`/`Deserialize` (its fields are
// `blob: Vec<u8>` + `lineage: SnapshotLineage` + `trace_bank: BankSnapshot`),
// so `EmulatorSnapshotBlob` is a transparent newtype that round-trips through
// the workspace `serde_json` exactly the way the inner `Snapshot` does. The
// inner `blob: Vec<u8>` becomes a base64 string at the JSON layer via the
// session's serde wrapper; the structured `lineage` and `trace_bank` fields
// stay structured so a `Session` JSON pretty-print is human-readable.
// F-A8 does not interpret the inner blob bytes.
```

`RomBlob` carries the original `.gb` cartridge bytes verbatim. F-A8 stores them once at `init` and never mutates them; they are the input to `Emulator::load_rom` on every `exec` and `inspect`. The `Session::rom_sha256` field is `sha256(rom.0)`; `Session::load` rechecks the equality and fails closed on mismatch.

`EmulatorSnapshotBlob` wraps F-A7's `Snapshot` type (which carries its own `SnapshotLineage`, including `rom_sha256: gbf_foundation::Hash256`, the post-load `BootMode` via `BootModeLineage`, the `policy_fingerprint: Hash256`, the `emu_version: EmuVersionTag`, and the `cycle_count: ClockCycles`). F-A8 does **not** call into `gameroy::Emulator::save_state()` directly; it goes through `gbf_emu::Emulator::snapshot() -> Result<Snapshot, EmuError>` so the lineage check on `Emulator::restore(&Snapshot) -> Result<(), EmuError>` is enforced by F-A7, not papered over by F-A8.

`SnapshotLineage::rom_sha256` is the typed `gbf_foundation::Hash256` newtype, not a raw `[u8; 32]`. F-A8's `Session::rom_sha256` stays at `[u8; 32]` for the on-disk wire format; the cross-check `Session::load` performs is `Hash256::from(session.rom_sha256) == session.emulator_snapshot.0.lineage.rom_sha256` (`SessionLoadError::SnapshotRomMismatch` on mismatch).

The base64 inflation on the inner `blob` is ~33%, but zstd recovers most of that on the wire, and the durability win of "the entire debugger state is in one self-contained file with cross-checked lineage" is worth the storage overhead.

The ROM-bytes story is owned by `RomBlob`, not by the snapshot blob — even though F-A7's `SnapshotLineage` also carries `rom_sha256`, `Session::rom_sha256` is the canonical session-level identity and the lineage check is an additional cross-validation, not a substitute.

#### 3.1.7 `TraceRing`

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRing {
    pub capacity: u32,
    pub events:   VecDeque<TraceEventPersisted>,    // capped at `capacity`
    pub dropped:  u64,                              // count of events dropped due to capacity
    pub next_seq: u64,                              // next sequence number to assign; not reset by clear_trace
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEventPersisted {
    pub seq:      u64,                  // monotone within session lifetime; see `TraceRing::next_seq`
    pub kind:     TraceEventKind,
    pub addr:     u16,
    pub data:     Vec<u8>,              // small; for memory events it's the touched bytes
    pub pc_at:    u16,
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
```

Default capacity is `1024` events. The ring is implemented over `VecDeque` (`push_back` / `pop_front`) so eviction is O(1). The JSON serialization writes `events` in head-to-tail order; the `dropped` counter records how many events were lost to capacity since the last `clear_trace()`.

The `serde` representation of `VecDeque<T>` is identical to `Vec<T>`, so the on-disk JSON does not depend on the deque layout — which is what we want for determinism.

`TraceEventPersisted` is a *persistence* type, deliberately decoupled from `gbf_emu::NormalizedTraceEvent`. F-A8's CLI converts at the boundary so a future `gbf-emu` change to `NormalizedTraceEvent` does not silently bump `SCHEMA_VERSION`; the conversion either keeps mapping into the existing `TraceEventPersisted` shape (potentially with a `Typed` catch-all variant) or the F-A8 maintainer chooses to bump `SCHEMA_VERSION` deliberately.

#### 3.1.8 Acceptance criteria (T-A8.1)

```bash
cargo test -p gbf-debug -- session::magic_round_trip
cargo test -p gbf-debug -- session::flags_must_be_zero
cargo test -p gbf-debug -- session::schema_version_pinned
cargo test -p gbf-debug -- session::schema_mismatch_is_fatal
cargo test -p gbf-debug -- session::serde_round_trip
cargo test -p gbf-debug -- session::write_then_read_byte_identical
cargo test -p gbf-debug -- session::trace_ring_capped
cargo test -p gbf-debug -- session::breakpoint_predicate_round_trip
cargo test -p gbf-debug -- session::watchpoint_kinds_exhaustive
```

#### 3.1.9 Acceptance criteria (T-A8.6)

```bash
cargo test -p gbf-debug -- symbols::from_sym_text_round_trip
cargo test -p gbf-debug -- symbols::sorted_canonical
cargo test -p gbf-debug -- symbols::duplicate_name_warned_not_fatal
cargo test -p gbf-debug -- symbols::unqualified_duplicate_is_ambiguous
cargo test -p gbf-debug -- symbols::resolve_in_bank_disambiguates
cargo test -p gbf-debug -- symbols::resolve_at_returns_name
```

#### 3.1.10 Acceptance criteria (T-A8.7)

```bash
cargo test -p gbf-debug -- predicate::stringified_round_trip
cargo test -p gbf-debug -- predicate::closure_is_invocation_local
cargo test -p gbf-debug -- predicate::closure_does_not_create_unconditional_persisted_breakpoint
cargo test -p gbf-debug -- predicate::closure_drop_is_warned
cargo test -p gbf-debug -- predicate::stringified_re_evaluated_on_next_exec
cargo test -p gbf-debug -- predicate::stringified_predicate_cannot_mutate_emulator
cargo test -p gbf-debug -- predicate::stringified_predicate_scope_is_read_only
```

### 3.2 `script.rs` — the rquickjs scripting host (T-A8.2)

#### 3.2.1 `ScriptHost` lifecycle

```rust
pub struct ScriptHost {
    runtime: rquickjs::Runtime,
    context: rquickjs::Context,
    deadline: Instant,
}

#[derive(Debug, Clone)]
pub struct ScriptConfig {
    pub timeout: Duration,                          // default Duration::from_secs(30) — wall-clock liveness watchdog only
    pub memory_limit_bytes: Option<usize>,          // None = unlimited; default Some(64 * 1024 * 1024)
    pub stack_limit_bytes:  Option<usize>,          // None = QuickJS default; default Some(1 * 1024 * 1024)
    pub snapshot_limit: u32,                        // default 32
    pub default_run_budget: gbf_emu::CycleBudget,   // default Machine(MCycles(1_000_000)) — applies when JS omits max_m_cycles
    pub max_step_instructions_per_call: u32,        // default 1_000_000 — bounds gb.step(n) so it cannot sit inside Rust past the watchdog
}

impl ScriptConfig {
    pub fn default() -> Self;
}

impl ScriptHost {
    pub fn new(config: ScriptConfig) -> Result<Self, ScriptError>;
    pub fn evaluate(
        &mut self,
        script_source: &str,
        gb_binding: GbBinding<'_>,
    ) -> Result<ScriptOutcome, ScriptError>;
}
```

A fresh `rquickjs::Runtime` is created per `evaluate` call. The runtime is dropped at the end of `evaluate`; there is no shared JS state between invocations. (rquickjs is `Send` for the Runtime but not `Sync`; F-A8 does not need either, all access is single-threaded.)

`ScriptConfig::default()` uses 30-second timeout, 64 MiB memory limit, 1 MiB JS stack. The defaults are tuned for "the agent is going to write a 100-line JS script that loops up to 10⁶ times against the emulator"; if a real workload pushes against either limit, the operator passes `--timeout` to the CLI (which constructs a non-default `ScriptConfig`).

#### 3.2.2 The wall-clock-deadline interrupt handler

QuickJS supports an interrupt handler hook:

```rust
runtime.set_interrupt_handler(Some(Box::new(move || {
    Instant::now() >= deadline
})));
```

Returning `true` aborts the running script with a JS-level "interrupted" exception, which `rquickjs::Context::with` propagates as `rquickjs::Error::Exception`. F-A8 catches that and wraps it as `ScriptError::Timeout { elapsed }`.

The interrupt handler runs every N opcodes (QuickJS sets N internally; we don't override it). At default settings, scripts that loop tightly check the deadline every few microseconds of wall-clock time, so a 30-second timeout fires within tens of milliseconds of the true deadline. This is precise enough; F-A8 does not promise sub-millisecond timeout accuracy.

#### 3.2.3 Memory limit

```rust
runtime.set_memory_limit(config.memory_limit_bytes.map(|n| n).unwrap_or(0));
```

QuickJS treats `0` as "no limit." A non-zero limit causes allocation failures inside the JS engine to surface as `JsError::OutOfMemory` (mapped to `ScriptError::OutOfMemory`).

#### 3.2.4 Determinism stubs: `Date.now`, `Math.random`, `console`

A script's behavior must not depend on wall-clock time or system entropy, because that would make `gbf-debug exec` non-reproducible. F-A8 patches the relevant globals at host setup:

```rust
ctx.globals().set("Date", date_stub_function_with_now_only())?;
let math: Object = ctx.globals().get("Math")?;
math.set("random", deterministic_random_fn)?;
ctx.globals().delete("console")?;
```

Specifically:

- `Date.now()` returns deterministic **virtual milliseconds since script start**, not host elapsed time. The source is F-A7 `ClockCycles` advanced by `gb.step` / `gb.run_until*` outcomes, converted using the DMG clock rate (4.194304 MHz; 1 ms = 4194 clocks, floor-rounded for stability). Before the first emulator-advancing call, `Date.now()` returns `0`. JS-only CPU loops do not advance virtual time. The wall-clock watchdog still exists as a host liveness guard, but its timing is never observable through `Date.now()`.
- `new Date(...)` throws `TypeError`. Only `Date.now()` exists.
- `Math.random()` returns deterministic pseudo-random values from a fixed-seeded `xorshift64*` generator. The seed is `0xdeadbeefcafebabe`; the seed does not depend on the input session or the script. Scripts that need a different stream may use their own JS-side PRNG with their own seed.
- Only `Math.random` is replaced; every other `Math.*` (sin, cos, sqrt, etc.) keeps QuickJS's IEEE-754 deterministic semantics. The `Math` object itself is not replaced wholesale.
- `console` is deleted (QuickJS exposes a minimal `console.log` by default in some configurations). The only blessed log path is `log(msg, data?)` (see §3.5.2).

The rquickjs runtime's own determinism is constrained to "no host-clock leak" by these stubs. The crate does not depend on `rquickjs`'s custom-allocator features (which would change memory-limit semantics per upstream docs); the `Cargo.toml` audit in §7 keeps the feature surface tight.

#### 3.2.5 Error reporting

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    SyntaxError { message: String, line: u32, column: u32 },
    RuntimeException { message: String, line: Option<u32>, column: Option<u32>, function: Option<String> },
    Timeout { elapsed_micros: u64 },
    OutOfMemory,
    StackOverflow,
    HostBindingError { method: String, source: String },   // e.g. gb.read called with negative addr
}
```

Every variant carries enough state to produce a `{kind, message, script_line, script_column}` JSON record (§3.5.5). The `function` field is filled when QuickJS provides a stack trace; it is `None` for top-level errors.

#### 3.2.6 Acceptance criteria (T-A8.2)

```bash
cargo test -p gbf-debug -- script::evaluate_returns_result
cargo test -p gbf-debug -- script::only_globalThis_result_is_captured
cargo test -p gbf-debug -- script::syntax_error_carries_position
cargo test -p gbf-debug -- script::runtime_exception_carries_position
cargo test -p gbf-debug -- script::timeout_aborts
cargo test -p gbf-debug -- script::date_now_is_virtual_time_not_wallclock
cargo test -p gbf-debug -- script::date_now_advances_with_emulator_clock_cycles
cargo test -p gbf-debug -- script::date_now_reproducible_across_runs
cargo test -p gbf-debug -- script::date_constructor_throws
cargo test -p gbf-debug -- script::math_random_is_seeded
cargo test -p gbf-debug -- script::math_other_methods_preserved
cargo test -p gbf-debug -- script::console_is_deleted
cargo test -p gbf-debug -- script::memory_limit_enforced
```

### 3.3 The `gb` object binding (T-A8.3)

This is the largest single surface in F-A8. The binding is implemented via rquickjs's `class!` macro, which binds a Rust struct to a JS prototype with typed methods.

#### 3.3.1 Surface

```js
gb.regs                                       // { pc, sp, a, b, c, d, e, h, l, f, bc, de, hl, ime }
                                              //   ime ∈ "disabled" | "enabled" | "to_be_enable" (mirrors F-A7 ImeSnapshot)
gb.read(addr, len)                            // -> Uint8Array; F-A7 peek_range, side-effect-free
gb.write(addr, bytes)                         // F-A7 poke loop, side-effect-free
gb.bus_read(addr, len)                        // -> Uint8Array; F-A7 bus_read, side-effecting
gb.bus_write(addr, bytes)                     // F-A7 bus_write, side-effecting
gb.step(n)                                    // n: u32; returns { pc_after, clock_cycles_consumed, m_cycles_floor_consumed }
gb.run_until(pc, max_m_cycles?)               // -> { reason: "pc_reached" | "max_cycles", ... }
gb.run_until_breakpoint(max_m_cycles?)        // -> { reason: "breakpoint" | "watchpoint" | "max_cycles", ... }

gb.add_breakpoint(addr, predicate?)           // predicate?: function | string expression
gb.remove_breakpoint(addr)
gb.list_breakpoints()                         // -> Array<{ addr, has_predicate, persisted_kind }>

gb.add_watchpoint(addr, kind, predicate?)     // kind: "read" | "write" | "rw"
gb.remove_watchpoint(addr, kind)
gb.list_watchpoints()                         // -> Array<{ addr, kind, has_predicate, persisted_kind }>

gb.snapshot()                                 // -> SnapshotHandle (opaque integer id)
gb.restore(handle)                            // restores emulator machine state to that snapshot

gb.symbol(name)                               // -> u16 | null; raises AmbiguousSymbol on banked duplicates
gb.symbol_in_bank(name, bank)                 // -> u16 | null
gb.symbol_at(addr)                            // -> string | null; raises AmbiguousSymbol on banked duplicates
gb.symbol_at_in_bank(addr, bank)              // -> string | null

gb.framebuffer()                              // -> Uint8Array (length = 160*144*1 = 23040, palette-indexed)
gb.input(buttons)                             // buttons: Array<"a"|"b"|"start"|"select"|"up"|"down"|"left"|"right">

gb.trace_ring()                               // -> Array<TraceEventJs>
gb.clear_trace()
```

Plus the two structured-output helpers (defined as global JS functions, not on `gb`):

```js
log(msg, data?)                               // appends to the structured log buffer
globalThis.result = expr                      // the host predefines `globalThis.result = null` before
                                              // evaluating the script, so `result = expr` (top-level
                                              // assignment to the implicit global) also updates
                                              // `globalThis.result`. `let result = expr` declares a
                                              // *new* lexical binding and does NOT update
                                              // `globalThis.result`; it is not captured.
```

#### 3.3.2 `gb.regs` — read-only snapshot

`regs` is a JS object property, not a method. Each access returns a fresh JS object whose fields are decoded from the emulator's register file at the moment of access. The 16-bit pseudo-fields (`bc`, `de`, `hl`) are computed from the 8-bit fields (`(b << 8) | c`, etc.). Writing to `gb.regs.a = 0x42` is **rejected** with a `TypeError` from the host binding — register writes go through `gb.write` (for memory) or are not exposed at all (for direct register manipulation, which is rarely needed and adds a determinism risk because it bypasses the gameroy state machine). If a future workload genuinely needs direct register manipulation, that's a follow-up bead.

The `ime` field is a string enum: `"disabled"`, `"enabled"`, or `"to_be_enable"`. This mirrors F-A7's `ImeSnapshot` exactly; collapsing it to `bool` would lose the observable post-`EI` pending-enable state.

#### 3.3.3 `gb.read(addr, len)` and `gb.write(addr, bytes)`

```rust
fn read(&self, addr: u16, len: u32) -> rquickjs::Result<Vec<u8>>;       // F-A7 peek_range (side-effect-free)
fn write(&mut self, addr: u16, bytes: Vec<u8>) -> rquickjs::Result<()>; // F-A7 poke loop  (side-effect-free)
fn bus_read(&mut self, addr: u16, len: u32) -> rquickjs::Result<Vec<u8>>;   // F-A7 bus_read  (side-effecting)
fn bus_write(&mut self, addr: u16, bytes: Vec<u8>) -> rquickjs::Result<()>; // F-A7 bus_write (side-effecting)
```

`read` rejects ranges where `u32::from(addr) + len > 0x1_0000` with `HostBindingError::AddressOverflow`, then delegates to F-A7 `peek_range`. It is side-effect-free and therefore rejects IO and other unsupported raw-backed regions with `HostBindingError::DebugMemoryUnsupported`. It does not advance the clock, does not emit guest trace events, and does not trigger guest traps.

`write` rejects ranges where `u32::from(addr) + bytes.len() as u32 > 0x1_0000`, then delegates to F-A7 `poke` byte-by-byte. Same side-effect-free contract as `read`.

`bus_read` / `bus_write` are explicit side-effecting operations. They delegate to F-A7 `bus_read` / `bus_write`, may advance clock state, may affect IO/MBC registers, and record `TraceOrigin::HostBus` in the F-A7 trace stream. This split is mandatory; F-A8 must not hide side-effecting bus access behind the neutral name `read`. Scripts that want "what does the CPU see at address X right now" use `read`; scripts that want "perform a CPU-style read at address X with all attendant side effects" use `bus_read`.

`Vec<u8>` round-trips as `Uint8Array` in rquickjs.

#### 3.3.4 `gb.step(n)` and `gb.run_until(pc, max_m_cycles?)`

```rust
fn step(&mut self, n: u32) -> rquickjs::Result<StepOutcome>;
fn run_until(&mut self, pc: u16, max_m_cycles: Option<u64>) -> rquickjs::Result<RunOutcome>;
fn run_until_breakpoint(&mut self, max_m_cycles: Option<u64>) -> rquickjs::Result<RunOutcome>;

#[derive(Serialize)]
pub struct StepOutcome {
    pub pc_after: u16,
    pub clock_cycles_consumed:    String,    // decimal u64 of F-A7 ClockCycles, no JS precision loss
    pub m_cycles_floor_consumed:  String,    // decimal u64 of F-A7 MCycles
}

#[derive(Serialize)]
pub struct RunOutcome {
    pub reason: RunStopReason,
    pub pc_at_stop: u16,
    pub clock_cycles_consumed:    String,    // decimal u64
    pub m_cycles_floor_consumed:  String,    // decimal u64
    pub trap_id: Option<u32>,                // BreakpointId.0 when reason is Breakpoint or Watchpoint
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStopReason {
    PcReached,
    Breakpoint,
    Watchpoint,
    MaxCyclesExceeded,
}
```

`run_until_breakpoint` is the natural primitive for "run until *any* registered F-A7 trap fires." It is implemented by calling F-A7 `Emulator::run_for(budget) -> Result<RunOutcome, EmuError>` and translating the resulting `RunOutcome::TrapHit { trap_id, kind, observed }` into `RunStopReason::Breakpoint` / `Watchpoint` (deciding which by inspecting `kind: TrapKind`). The other `RunOutcome` variants `BudgetElapsed { observed, requested }` and `Idle { state, observed }` map to `RunStopReason::MaxCyclesExceeded` and `RunStopReason::PcReached`-with-no-pc-match respectively.

`run_until(pc, ...)` delegates to F-A7 `Emulator::run_until_pc(pc, budget) -> Result<RunOutcome, EmuError>`. F-A8 does not install its own anonymous trap for this path; F-A7 already owns the correct pre-instruction PC-trap semantics for `run_until_pc` (it uses the reserved `BreakpointId::RUN_UNTIL_PC = u32::MAX` internally), so duplicating that here would risk double-firing. F-A8's persisted breakpoint map is untouched by `run_until(pc, ...)`.

PC traps fire **before** executing the instruction at the registered address. Memory watchpoints are **post-instruction** under F-A7's M0 `io_trace` backend; `regs` visible to a memory-watchpoint predicate reflects post-instruction state. Exact pre-access memory watchpoints are not part of F-A8 M0 and are not silently emulated by sandwiching a `step()`; predicates that need pre-access state must be written as PC traps near the access site.

Both `run_until*` functions always have an effective `max_m_cycles`: either the explicit JS argument or `ScriptConfig::default_run_budget` (default `CycleBudget::Machine(MCycles(1_000_000))`). This is the **deterministic semantic guard** that prevents a host-side emulator loop from bypassing the QuickJS interrupt handler — the QuickJS wall-clock watchdog cannot interrupt a long-running Rust host call while that call is executing inside F-A7's `run_for`, so a script that called `gb.run_until_breakpoint()` with no cap could hang inside Rust outside QuickJS opcode dispatch. The budget closes that hole.

`gb.step(n)` is **F-A8 territory**. F-A7's `Emulator::step(&mut self) -> Result<StepOutcome, EmuError>` is single-shot (no `n` parameter); F-A8 implements the JS-visible `gb.step(n)` by invoking F-A7 `Emulator::step()` `n` times in a host-side loop, accumulating `clock_cycles_consumed` from successive `StepOutcome` values. `n` is bounded by `ScriptConfig::max_step_instructions_per_call` (default `1_000_000`) so a script that wrote `gb.step(1_000_000_000_000)` cannot bypass the watchdog by sitting inside a single Rust call for hours; F-A8 also checks the QuickJS interrupt deadline between successive `step()` calls so the wall-clock watchdog still fires inside a tight `gb.step(1e6)` invocation.

If the cap is hit before any other stop condition, the outcome is `RunStopReason::MaxCyclesExceeded` and the agent decides whether to keep going. The wall-clock timeout in `ScriptConfig::timeout` remains as a process liveness guard for pathological JS-only loops; it does not appear in any deterministic output.

#### 3.3.5 Breakpoints and watchpoints

The JS-side methods translate to F-A7 trap-dispatcher calls. The host maintains two parallel structures:

- a `BTreeMap<u16, BreakpointPersisted>` of *persisted* breakpoints (those that survive across `exec` invocations), and
- a `BTreeMap<u32, ClosurePredicate>` of *closure-shaped* predicates keyed by trap id, alive only for the current invocation.

When the script calls `gb.add_breakpoint(addr, fn)` with a closure, the host:

1. wraps the JS closure in a Rust `Box<TrapPredicate>` that calls back into QuickJS, then registers a PC trap with `Emulator::traps().add_pc(addr, Predicate::Closure(boxed), TrapAction::HaltAndReport)`, capturing the returned `BreakpointId`;
2. stores the closure (and the `BreakpointId`) in the closure-predicate map (invocation-local only);
3. records a `Warning { kind: "predicate_not_persisted", ... }` so the agent learns that the closure did not survive the invocation;
4. **does not insert anything into the persisted breakpoint map.** The trap is removed via `traps().remove(id)` at script end (the entire `Emulator` value, including its dispatcher, is dropped between invocations anyway), so the next `exec` starts without it.

When the script calls `gb.add_breakpoint(addr, "expr")` with a string, the host:

1. parses the string as a JS expression at registration time (early-error catches syntax problems);
2. registers a PC trap with the F-A7 dispatcher. F-A8 does not register `Predicate::Source(s)` directly — F-A7's `predicate_matches` returns `TrapPredicateError::SourceRequiresEvaluator` for `Source` because the dispatcher cannot itself host a JS evaluator. Instead F-A8 wraps the source string in a Rust closure that re-evaluates the expression in QuickJS on every trap fire, and registers it as `Predicate::Closure(boxed)` with `action: TrapAction::HaltAndReport`. The original source string is retained in F-A8's persisted-breakpoint map so the next `exec` can rebuild the closure;
3. inserts a `BreakpointPersisted { addr, predicate: PersistedPredicate::StringifiedSource(s), enabled: true }`;
4. on every trap fire the Rust closure re-evaluates the expression in a fresh **read-only predicate scope** mirroring F-A7's `TrapContext` (`{ regs, pc, access, cycle, view }`) plus two F-A8-injected helpers `symbol(name)` / `symbolInBank(name, bank)` that resolve through the in-session `SessionSymbolTable`. F-A7's `TrapContext` itself does not carry `symbol` / `symbolInBank`; those are F-A8 additions to the predicate scope. Mutating `gb.*` methods are not in scope and any reference to them throws `ReferenceError`.

`list_breakpoints()` returns both: each entry has a `persisted_kind` field of `"none" | "stringified"` and a `has_predicate` boolean that is true iff a closure-predicate is alive in the current invocation.

`remove_breakpoint(addr)` removes both entries (persisted + closure-predicate, if any) and calls `Emulator::traps().remove(id)` for the underlying dispatcher entry.

Watchpoints are analogous, with the additional `kind: WatchpointKind` parameter and the `add_mem_read` / `add_mem_write` / `add_mem_rw` dispatch above. Each watchpoint takes a single address, which F-A8 wraps in `AddressRange::new(addr, addr)?` before calling the F-A7 method (single-byte ranges are valid).

#### 3.3.6 `gb.snapshot()` and `gb.restore(handle)` — in-script transient branching

```rust
fn snapshot(&mut self) -> rquickjs::Result<u32>;        // returns SnapshotHandle
fn restore(&mut self, handle: u32) -> rquickjs::Result<()>;
```

`snapshot()` calls `gbf_emu::Emulator::snapshot()` to produce a `Snapshot` that **excludes F-A8 debug traps, closure predicates, and persisted breakpoint/watchpoint maps** by construction (F-A8's dispatcher state lives outside the `Emulator` value, not inside it). The snapshot is stored in a `BTreeMap<u32, Snapshot>` keyed by a monotone counter, and the counter is returned as a `u32` to JS.

`restore(handle)` looks up the snapshot in the map, calls `Emulator::restore(&snapshot)` (which re-applies machine state and validates `SnapshotLineage`), and then **re-applies the current invocation's debugger traps from F-A8-owned maps** so the post-restore state matches the breakpoint/watchpoint surface the script set up before the snapshot. The snapshot is *retained* in the map so the same handle can be restored multiple times. Snapshots are dropped only when the JS context is dropped (at script end).

The snapshot map has a soft cap (default 32 snapshots) to prevent runaway memory use in long scripts; exceeding the cap returns `HostBindingError::SnapshotLimitExceeded`. The cap is configurable via `ScriptConfig::snapshot_limit`.

In-script snapshots **do not** end up in the on-disk session. The on-disk session lineage is one-hop `parent_sha256`; in-script branching is a within-invocation concern.

#### 3.3.7 `gb.symbol(name)`, `gb.symbol_in_bank(name, bank)`, `gb.symbol_at(addr)`, `gb.symbol_at_in_bank(addr, bank)`

All four delegate to `SessionSymbolTable::resolve`, `resolve_in_bank`, `resolve_at`, and `resolve_at_in_bank`. Returning `null` (JS) maps to `Option::None` on the Rust side. The methods do not consult the original `.sym` file on disk; the in-session embedded table is authoritative.

`gb.symbol(name)` raises `HostBindingError::AmbiguousSymbol { name, candidates }` when the underlying `SessionSymbolTable::resolve` returns `Err(SymbolResolutionError::AmbiguousName { ... })`. The script must call `gb.symbol_in_bank(name, bank)` to disambiguate. `gb.symbol_at(addr)` analogously raises on multi-bank ambiguity, and `gb.symbol_at_in_bank(addr, bank)` is the explicit disambiguator.

#### 3.3.8 `gb.framebuffer()` and `gb.input(buttons)`

`framebuffer()` returns the current 160×144 palette-indexed framebuffer as a 23040-byte `Uint8Array`. The exact pixel format is row-major, top-left to bottom-right, one palette index byte per pixel (`0..=3`); the layout follows F-A7's `Emulator::framebuffer` shape and F-A8 does not re-pack.

`input(buttons)` **replaces** the current pressed-button set: it does not toggle. Passing `[]` releases all buttons. The state persists across emulator steps until the next `input` call, which matches the gameroy joypad model.

`buttons: Array<"a"|"b"|"start"|"select"|"up"|"down"|"left"|"right">` — unknown button names raise `HostBindingError::UnknownButton`.

#### 3.3.9 `gb.trace_ring()` and `gb.clear_trace()`

`trace_ring()` returns the current trace ring as a JS array, with each entry shaped as:

```js
{
  seq: string,           // decimal u64 as string, no JS number-precision loss
  kind: string,          // "mem_read" | "mem_write" | "breakpoint_hit" | "watchpoint_hit" | "step_boundary"
  addr: number,
  data: Uint8Array,
  pc_at: number,
}
```

`clear_trace()` empties the ring and resets the dropped counter. It does **not** reset `TraceRing::next_seq`, so sequence numbers remain monotone across the entire session lifetime even after one or more clears.

#### 3.3.10 Acceptance criteria (T-A8.3)

```bash
cargo test -p gbf-debug -- gb::regs_round_trip
cargo test -p gbf-debug -- gb::regs_write_rejected
cargo test -p gbf-debug -- gb::regs_ime_is_tri_state_string
cargo test -p gbf-debug -- gb::read_rejects_overflow
cargo test -p gbf-debug -- gb::write_rejects_overflow
cargo test -p gbf-debug -- gb::read_is_side_effect_free
cargo test -p gbf-debug -- gb::bus_read_emits_host_bus_trace
cargo test -p gbf-debug -- gb::bus_write_emits_host_bus_trace
cargo test -p gbf-debug -- gb::step_returns_pc_and_cycles
cargo test -p gbf-debug -- gb::step_cycle_fields_are_decimal_strings
cargo test -p gbf-debug -- gb::step_n_capped_by_max_step_instructions_per_call
cargo test -p gbf-debug -- gb::watchpoint_predicate_sees_post_instruction_state
cargo test -p gbf-debug -- gb::run_until_pc_stops
cargo test -p gbf-debug -- gb::run_until_max_cycles_caps
cargo test -p gbf-debug -- gb::run_until_without_explicit_cap_uses_default_budget
cargo test -p gbf-debug -- gb::run_until_one_shot_trap_is_not_persisted
cargo test -p gbf-debug -- gb::host_call_cannot_bypass_watchdog_budget
cargo test -p gbf-debug -- gb::add_breakpoint_closure_invocation_local
cargo test -p gbf-debug -- gb::add_breakpoint_string_round_trips
cargo test -p gbf-debug -- gb::watchpoint_kinds_exhaustive
cargo test -p gbf-debug -- gb::snapshot_restore_branching
cargo test -p gbf-debug -- gb::snapshot_excludes_debug_traps
cargo test -p gbf-debug -- gb::snapshot_limit_enforced
cargo test -p gbf-debug -- gb::symbol_resolves
cargo test -p gbf-debug -- gb::symbol_ambiguous_raises
cargo test -p gbf-debug -- gb::symbol_in_bank_disambiguates
cargo test -p gbf-debug -- gb::symbol_at_resolves
cargo test -p gbf-debug -- gb::symbol_at_ambiguous_raises
cargo test -p gbf-debug -- gb::symbol_at_in_bank_disambiguates
cargo test -p gbf-debug -- gb::framebuffer_shape
cargo test -p gbf-debug -- gb::input_empty_releases_all_buttons
cargo test -p gbf-debug -- gb::input_unknown_button_rejected
cargo test -p gbf-debug -- gb::trace_ring_shape
cargo test -p gbf-debug -- gb::trace_ring_seq_is_string_in_js
```

### 3.4 `cli.rs` — `init` / `exec` / `inspect` (T-A8.5)

#### 3.4.1 The binary target

`gbf-debug/Cargo.toml` declares:

```toml
[[bin]]
name = "gbf-debug"
path = "src/bin/gbf-debug.rs"
```

The binary is a thin `clap`-driven dispatcher that calls into `gbf_debug::cli::{run_init, run_exec, run_inspect}`. The library exposes those three functions so tests can drive them without going through process spawn.

```rust
pub fn run_init(args: InitArgs) -> Result<InitOutcome, CliError>;
pub fn run_exec(args: ExecArgs) -> Result<ExecOutcome, CliError>;
pub fn run_inspect(args: InspectArgs) -> Result<InspectOutcome, CliError>;

pub struct InitArgs {
    pub rom_path: PathBuf,
    pub sym_path: Option<PathBuf>,    // optional; if absent, an empty symbol table is embedded
    pub out_path: PathBuf,
    pub trace_capacity: u32,           // default 1024
}

pub struct ExecArgs {
    pub in_path: PathBuf,
    pub script_path: PathBuf,
    pub out_path: PathBuf,
    pub timeout: Duration,                              // default Duration::from_secs(30) — wall-clock liveness only
    pub default_run_budget: gbf_emu::CycleBudget,       // default Machine(MCycles(1_000_000))
    pub max_step_instructions_per_call: u32,            // default 1_000_000
    pub emit_metrics: bool,                             // false → omit `metrics` from envelope (golden-test default)
    pub write_partial_on_timeout: bool,                 // false → no partial session on watchdog timeout
    pub replace_existing_out: bool,                     // false → reject if out_path exists
}

pub struct InspectArgs {
    pub in_path: PathBuf,
}
```

#### 3.4.2 `init`

Sequence:

1. Read `rom_path` to bytes; compute `rom_sha256`.
2. Construct an `Emulator` via F-A7 with `EmulatorConfig { boot_mode: BootMode::PostBootDmg, policy: DeterminismPolicy::default(), .. }` and call `load_rom(rom_bytes)`.
3. If `sym_path` is `Some`, read the file, call `SessionSymbolTable::from_sym_text(input)` and capture the returned `SymbolHydration { table, warnings }`. If `None`, use `(SessionSymbolTable::default(), Vec::new())`.
4. Assert the post-load register snapshot has `pc == 0x0100`. Mismatch surfaces as `CliError::PostLoadPcUnexpected { observed }` (exit code 6).
5. Capture the emulator's initial F-A7 snapshot via `Emulator::snapshot()` → `EmulatorSnapshotBlob`.
6. Build `Session { schema_version: 1, parent_sha256: None, rom_sha256, rom: RomBlob(rom_bytes), emulator_snapshot, symbols, breakpoints: vec![], watchpoints: vec![], trace_ring: TraceRing::new(args.trace_capacity), metadata: SessionMetadata { abi_version_observed: None, created_at_micros_since_init: 0, notes: BTreeMap::new() } }`.
7. Write the session to `out_path` via `Session::write_new(out_path)` (rejecting an existing path unless `--replace-existing-out` was passed, in which case `Session::replace(out_path)`). Both return the SHA-256 of the written bytes.
8. Emit the `InitEnvelope` (§3.5.3) — including any hydration `warnings` collected in step 3 — to stdout.

The "PC at `$0100`" promise from planv0.md is an **explicit F-A8 invariant**, not implicit emulator behavior. The check lives in step 4 above and is gated by `cli::init_post_load_pc_is_0x0100`; if F-A7's load mode ever changes (e.g., starts at the bootrom entrypoint instead), this test catches it before the session is written.

#### 3.4.3 `exec`

Sequence:

1. Reject `in_path == out_path` with `CliError::InOutSamePath` (exit code 1). In-place mutation destroys the parent/child session model and interacts badly with crash recovery; users who want a fork overwrite must write a new path and `mv` it themselves.
2. Read `in_path` bytes; compute the bytes' SHA-256 → `parent_sha256_for_output`.
3. Call `Session::load_bytes(input_bytes)` to validate magic/flags/schema, the `RomBlob` ↔ `rom_sha256` integrity, and the `Snapshot` lineage cross-check.
4. Construct an `Emulator` via F-A7 with `EmulatorConfig { boot_mode: BootMode::PostBootDmg, policy: DeterminismPolicy::default(), .. }`, call `load_rom(session.rom.0)`, then `Emulator::restore(&session.emulator_snapshot.0)`. Restore failures from F-A7 lineage checks surface as typed `CliError::EmulatorRestore`.
5. For each persisted breakpoint, register a PC trap with the dispatcher and (if the predicate is `StringifiedSource`) compile the predicate now to catch syntax errors before script start. Stringified-predicate compilation failures abort `exec` with `CliError::PredicateCompileFailed { addr, source, error }`.
6. For each persisted watchpoint, register a memory trap via F-A7 `TrapDispatcher::add_mem_read` / `add_mem_write` / `add_mem_rw` (F-A7 names the read/write union `MemRw`, not `MemReadWrite`). The single watchpoint address is wrapped in `AddressRange::new(addr, addr)?` before the call.
7. Build the `GbBinding` over the emulator + session + closure-predicate map (initially empty for the new invocation).
8. Read `script_path` bytes; pass to `ScriptHost::evaluate(script_source, gb_binding)`.
9. On `Ok(outcome)`: capture the emulator's post-script `Snapshot`, build the new `Session` with `parent_sha256: Some(parent_sha256_for_output)`, write to `out_path` via `write_new` or `replace` per `--replace-existing-out`, emit `ExecEnvelope` (§3.5.2) with the script's `result`, `logs`, and `warnings` to stdout.
10. On deterministic script errors (`SyntaxError`, `RuntimeException`, `HostBindingError`, predicate compile/eval failures): write the *post-error* session anyway (the emulator may have advanced before the error fired), emit `ErrorEnvelope` (§3.5.5) to stderr, exit with the appropriate non-zero code.
11. On wall-clock watchdog timeout: by default write **no** normal output session. If `--write-partial-on-timeout` was passed, write a partial session, set `ErrorEnvelope.determinism = "nondeterministic_partial"`, and include `partial_session_path` + `partial_session_sha256` in the envelope. The partial session is for forensic inspection only and is excluded from determinism golden tests.

The "write the post-error session anyway" rule for deterministic errors is deliberate: a script that runs for 1000 steps and then throws on step 1001 has produced 1000 steps of useful state. The agent inspects the resulting session to see how far the script got. Watchdog timeouts are *not* deterministic errors, which is why their session is opt-in and explicitly marked.

#### 3.4.4 `inspect`

Sequence:

1. Read `in_path` bytes; compute SHA-256.
2. Call `Session::load_bytes(input_bytes)`.
3. Construct an emulator via F-A7 with `EmulatorConfig { boot_mode: BootMode::PostBootDmg, policy: DeterminismPolicy::default(), .. }`, call `load_rom(session.rom.0)`, then `Emulator::restore(&session.emulator_snapshot.0)` so the dump can include current register state.
4. Emit `InspectEnvelope` (§3.5.4) to stdout.

`inspect` writes nothing to disk and does not run any script. It is safe to run repeatedly.

#### 3.4.5 Exit codes

| Exit code | Meaning |
|-----------|---------|
| 0         | Success; envelope on stdout |
| 1         | Bad CLI arguments; `ErrorEnvelope { kind: "cli_args", ... }` on stderr |
| 2         | Session load failed (`SessionLoadError`); `ErrorEnvelope` on stderr |
| 3         | Session write failed (`SessionWriteError`); `ErrorEnvelope` on stderr |
| 4         | Script error (`ScriptError`, including `Timeout`, `OutOfMemory`, syntax/runtime); `ErrorEnvelope` on stderr |
| 5         | Predicate compile failed (`CliError::PredicateCompileFailed`); `ErrorEnvelope` on stderr |
| 6         | I/O / post-load PC mismatch (`CliError::Io`, `CliError::PostLoadPcUnexpected`); `ErrorEnvelope` on stderr |
| 7         | Symbol-table hydration error (`SymbolHydrationError`); `ErrorEnvelope` on stderr |

The binary disables clap's default human-prose error rendering (`clap::Command::disable_help_flag(false).color(clap::ColorChoice::Never)` plus a top-level `clap::Error` → `ErrorEnvelope` translator). Even `--help` output is structured: a `--help` invocation emits an `ErrorEnvelope { kind: "help", message: "<flat usage>" , ... }` to stdout with exit code 0. There is no mode where `gbf-debug` writes human prose.

#### 3.4.6 Acceptance criteria (T-A8.5)

```bash
cargo test -p gbf-debug -- cli::init_writes_session
cargo test -p gbf-debug -- cli::init_no_sym_uses_empty_table
cargo test -p gbf-debug -- cli::init_envelope_shape
cargo test -p gbf-debug -- cli::init_post_load_pc_is_0x0100
cargo test -p gbf-debug -- cli::exec_runs_script
cargo test -p gbf-debug -- cli::exec_persists_breakpoints
cargo test -p gbf-debug -- cli::exec_writes_session_on_script_error
cargo test -p gbf-debug -- cli::exec_envelope_shape
cargo test -p gbf-debug -- cli::exec_parent_sha256_matches_input
cargo test -p gbf-debug -- cli::exec_restores_without_rom_or_sym_sidecar
cargo test -p gbf-debug -- cli::session_contains_original_rom_bytes
cargo test -p gbf-debug -- cli::rom_sha256_matches_embedded_rom_blob
cargo test -p gbf-debug -- cli::inspect_no_run_state_dump
cargo test -p gbf-debug -- cli::inspect_envelope_shape
cargo test -p gbf-debug -- cli::e2e_init_exec_inspect
cargo test -p gbf-debug -- cli::deterministic_two_runs_byte_identical
cargo test -p gbf-debug -- cli::watchdog_timeout_does_not_write_deterministic_session
cargo test -p gbf-debug -- cli::watchdog_timeout_partial_marked_nondeterministic
cargo test -p gbf-debug -- cli::write_partial_on_timeout_writes_partial_session
cargo test -p gbf-debug -- cli::arg_error_emits_json_error_envelope
cargo test -p gbf-debug -- cli::arg_error_emits_no_human_prose
cargo test -p gbf-debug -- cli::exec_in_out_same_path_rejected
cargo test -p gbf-debug -- cli::write_new_rejects_existing_out
cargo test -p gbf-debug -- cli::replace_existing_out_replaces
cargo test -p gbf-debug -- cli::exec_rejects_rom_hash_mismatch
cargo test -p gbf-debug -- cli::exec_rejects_snapshot_lineage_mismatch
cargo test -p gbf-debug -- session::atomic_write_replace_semantics
```

### 3.5 Structured CLI output (T-A8.4)

#### 3.5.1 Envelope discipline

Every CLI command emits exactly one JSON object on stdout (success) or stderr (failure), **including argument-parse failures**. The binary disables clap's default human-prose error rendering and maps `clap::Error` into `ErrorEnvelope { kind: "cli_args", ... }`. There is no human-prose CLI output, no progress bar, no banner. The `inspect` envelope is the same shape as `exec` minus the script-execution fields.

#### 3.5.2 `ExecEnvelope`

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ExecEnvelope {
    pub command: &'static str,           // "exec"
    pub result: serde_json::Value,        // the script's `globalThis.result`, canonicalized; or null
    pub logs: Vec<LogEntry>,              // ordered
    pub session_path: String,             // exact UTF-8 CLI argument as passed, not canonicalized
    pub session_sha256: String,           // hex
    pub parent_sha256: Option<String>,    // hex; None for sessions produced by init
    pub warnings: Vec<Warning>,           // non-fatal issues (predicate not persisted, trace overflow, ...)
    pub metrics: Option<ExecMetrics>,     // present only with `--emit-metrics`; excluded from determinism golden tests
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub message: String,
    pub data: serde_json::Value,                      // canonicalized
    pub ts_micros_since_script_start: u64,            // virtual time, not host time
}

#[derive(Debug, Clone, Serialize)]
pub struct ExecMetrics {
    pub script_micros: u64,
    pub host_setup_micros: u64,
    pub session_write_micros: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Warning {
    pub kind: String,     // "predicate_not_persisted", "trace_overflow", "duplicate_symbol_name", ...
    pub detail: serde_json::Value,                    // canonicalized
}
```

The `result` field is produced by `js_value_to_canonical_json(script_result)`. Before envelope serialization, every `serde_json::Value` produced from JS is recursively canonicalized:

- object keys sorted lexicographically;
- `undefined`, functions, and Symbols become JSON `null` plus a warning;
- `NaN`, `Infinity`, and `-Infinity` become JSON `null` plus a warning;
- `BigInt` becomes a decimal string plus a warning unless the caller explicitly requests lossy numeric conversion;
- cyclic structures become JSON `null` plus a warning.

The canonicalization step runs whether or not the workspace's `serde_json` ends up with the `preserve_order` feature enabled, so envelope output is byte-stable regardless of feature unification.

The `logs` array is appended to in `log(msg, data?)` order. `data` defaults to JSON `null` if the second argument is omitted and is canonicalized with the same rules as `result`.

`ts_micros_since_script_start` is sourced from the same deterministic virtual clock that drives `Date.now()` (§3.2.4); the values agree and are byte-stable across repeated runs. Host-duration metrics live in the opt-in `metrics` field and are excluded from the determinism golden test.

#### 3.5.3 `InitEnvelope`

```rust
#[derive(Debug, Clone, Serialize)]
pub struct InitEnvelope {
    pub command: &'static str,           // "init"
    pub session_path: String,            // exact UTF-8 CLI argument as passed
    pub session_sha256: String,
    pub rom_sha256: String,
    pub symbol_count: u32,
    pub warnings: Vec<Warning>,           // e.g. duplicate_symbol_name
}
```

#### 3.5.4 `InspectEnvelope`

```rust
#[derive(Debug, Clone, Serialize)]
pub struct InspectEnvelope {
    pub command: &'static str,           // "inspect"
    pub session_path: String,            // exact UTF-8 CLI argument as passed
    pub session_sha256: String,
    pub schema_version: u32,
    pub parent_sha256: Option<String>,
    pub rom_sha256: String,
    pub regs: RegsSnapshot,               // decoded from the embedded save state
    pub breakpoints: Vec<BreakpointPersisted>,
    pub watchpoints: Vec<WatchpointPersisted>,
    pub trace_ring_summary: TraceRingSummary,
    pub symbols_summary: SymbolsSummary,
    pub metadata: SessionMetadata,
}

#[derive(Debug, Clone, Serialize)]
pub struct RegsSnapshot {
    pub pc: u16, pub sp: u16,
    pub a: u8, pub b: u8, pub c: u8, pub d: u8, pub e: u8,
    pub h: u8, pub l: u8, pub f: u8,
    pub ime: &'static str,    // "disabled" | "enabled" | "to_be_enable" — mirrors gbf_emu::ImeSnapshot tri-state
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
```

`inspect` does not emit the full trace ring contents (potentially many KB) or the full symbol table; it emits summaries. A future `inspect --full` flag could dump everything; F-A8 does not ship that flag.

#### 3.5.5 `ErrorEnvelope`

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub command: String,                 // "init" | "exec" | "inspect" | "" (for cli_args before subcommand parses)
    pub kind: String,                    // "cli_args" | "session_load" | "script_syntax" | "watchdog_timeout" | ...
    pub message: String,
    pub script_line: Option<u32>,
    pub script_column: Option<u32>,
    pub script_function: Option<String>,
    pub session_path: Option<String>,             // exact UTF-8 CLI argument as passed
    pub partial_session_path: Option<String>,     // for exec, only set when --write-partial-on-timeout fires
    pub partial_session_sha256: Option<String>,   // for exec, the post-error session if it was written
    pub determinism: Option<String>,              // "nondeterministic_partial" for watchdog-aborted sessions, otherwise absent
}
```

Errors are written to stderr as one JSON object. Exit code follows §3.4.5.

`Warning` (used in success envelopes) and `ErrorEnvelope` (used on failure) are intentionally separate types: a `Warning` does not change the exit code, while an `ErrorEnvelope` always accompanies a non-zero exit.

#### 3.5.6 Acceptance criteria (T-A8.4)

```bash
cargo test -p gbf-debug -- output::exec_envelope_serializes
cargo test -p gbf-debug -- output::init_envelope_serializes
cargo test -p gbf-debug -- output::inspect_envelope_serializes
cargo test -p gbf-debug -- output::error_envelope_serializes
cargo test -p gbf-debug -- output::log_entries_ordered
cargo test -p gbf-debug -- output::ts_micros_monotone
cargo test -p gbf-debug -- output::result_not_serializable_warns
cargo test -p gbf-debug -- output::warning_does_not_change_exit_code
cargo test -p gbf-debug -- output::default_envelope_contains_no_host_timing
cargo test -p gbf-debug -- output::date_now_and_log_timestamps_are_byte_stable
cargo test -p gbf-debug -- output::js_object_result_keys_are_sorted
cargo test -p gbf-debug -- output::log_data_keys_are_sorted
cargo test -p gbf-debug -- output::nan_and_infinity_warn
cargo test -p gbf-debug -- output::cyclic_result_warns
cargo test -p gbf-debug -- output::js_value_canonicalization_recursive
```

### 3.6 `lib.rs` — re-exports, feature gates, `unsafe`-free assertion

```rust
//! Agent-facing scripted debugger CLI for Game Boy ROMs.
//!
//! See `history/rfcs/F-A8-gbf-debug.md` for design rationale.

#![forbid(unsafe_code)]

pub mod session;
pub mod script;
pub mod cli;

pub use session::{
    Session, SessionLoadError, SessionWriteError, SessionMetadata,
    SessionSymbolTable, SessionSymbolEntry, SymbolHydration,
    SymbolHydrationError, SymbolResolutionError,
    BreakpointPersisted, WatchpointPersisted, WatchpointKind,
    PersistedPredicate, TraceRing, TraceEventPersisted, TraceEventKind,
    RomBlob, EmulatorSnapshotBlob, SCHEMA_VERSION,
};
pub use script::{ScriptHost, ScriptConfig, ScriptError, ScriptOutcome};
pub use cli::{
    run_init, run_exec, run_inspect,
    InitArgs, ExecArgs, InspectArgs,
    InitEnvelope, ExecEnvelope, InspectEnvelope, ErrorEnvelope,
    LogEntry, Warning, RegsSnapshot, TraceRingSummary, SymbolsSummary,
    ExecMetrics, CliError,
};

// Backwards-compatibility aliases for callers that prefer Outcome-shaped names.
pub type InitOutcome    = InitEnvelope;
pub type ExecOutcome    = ExecEnvelope;
pub type InspectOutcome = InspectEnvelope;
```

`#![forbid(unsafe_code)]` denies any `unsafe` token in the crate. The rquickjs upstream crate contains `unsafe` for FFI to QuickJS-NG, but that is encapsulated; F-A8 introduces zero `unsafe` lines.

There are no Cargo features. The crate builds in exactly one configuration. (A future `serde-json-canonical` or `panic-on-warn` feature might be useful; no need in M0.)

### 3.7 `.agents/skills/gbf-debug-usage/SKILL.md` — agent skill (T-A8.8a)

F-A8 ships an Agent Skill at `.agents/skills/gbf-debug-usage/SKILL.md` per the agentskills.io specification. The skill is the load-bearing handoff between "the CLI exists" and "the agent uses the CLI correctly." Without it, every agent debugging session re-derives the envelope shape, the predicate split, and the determinism contract from this RFC; with it, the agent loads ~100 tokens of frontmatter at startup, ~5 KB of body when the skill activates, and pulls in deeper references only when a specific subtask demands them.

#### 3.7.1 Directory layout

```
.agents/skills/gbf-debug-usage/
├── SKILL.md                           # required: frontmatter + body, < 500 lines
├── references/
│   ├── ENVELOPE.md                    # full ExecEnvelope/InitEnvelope/InspectEnvelope/ErrorEnvelope shapes
│   ├── GB_OBJECT.md                   # full gb.* surface (ported from §3.3 of this RFC)
│   ├── PREDICATES.md                  # closure vs stringified, the read-only TrapContext scope
│   ├── DETERMINISM.md                 # virtual clock, run budgets, watchdog policy
│   └── RECIPES.md                     # cross-reference index for assets/recipes/
└── assets/
    └── recipes/
        ├── run_to_entry.js            # gb.add_breakpoint(gb.symbol("entry")) + run_until_breakpoint
        ├── dump_regs_at_pc.js         # gb.run_until(0x0150) + result = { ... gb.regs }
        ├── memory_watchpoint.js       # gb.add_watchpoint(addr, "write", "...") + run + inspect
        ├── snapshot_branch.js         # gb.snapshot()/restore() bracketing two branches
        └── trace_io_writes.js         # gb.clear_trace() + run + result = gb.trace_ring()
```

The `assets/recipes/*.js` files are **executable end-to-end** against the F-A8 tiny-ROM e2e fixture (the same `.gb` produced by the §6.2 test ROM). The skill's acceptance test runs each recipe via `gbf-debug exec` and asserts the envelope shape; if a recipe drifts from the CLI surface, the test fails.

#### 3.7.2 `SKILL.md` frontmatter

```yaml
---
name: gbf-debug-usage
description: Drive the gbf-debug agent CLI to inspect, step, and script Game Boy ROM execution. Use when working with .gbsess session files, the gbf-debug binary, the gb JS object, breakpoint/watchpoint scripts, the F-A5 runtime nucleus boot, or any Game Boy ROM debugging task in this workspace.
license: Proprietary. LICENSE.txt has complete terms
compatibility: Requires the gbf-debug binary built from this workspace
metadata:
  feature: F-A8
  bead: bd-1aaz
---
```

The `name` is `gbf-debug-usage` (lowercase, hyphens only, matches the parent directory). The `description` is well under 1024 characters and includes the keywords an agent will plausibly search for: "gbf-debug", ".gbsess", "gb JS object", "breakpoint", "Game Boy".

#### 3.7.3 Body content (under 500 lines)

The `SKILL.md` body is structured for progressive disclosure. The top-level sections are:

1. **When to use** — one paragraph mapping common tasks to the right subcommand.
2. **The agent loop** — a single worked example: write `script.js`, run `gbf-debug exec --in s0.gbsess --script script.js --out s1.gbsess`, parse the envelope from stdout.
3. **The five rules that change behavior** — closure-vs-string predicates; `read` vs `bus_read`; `globalThis.result`; `gb.snapshot/restore` vs `cp`-ing the session; the typed `CycleBudget` cap on `run_until*`.
4. **Envelope contract** — short table pointing at `references/ENVELOPE.md` for full shapes.
5. **Determinism gotchas** — `Date.now()` is virtual time; watchdog timeouts do not write a normal output session; argument errors are JSON, not clap prose.
6. **Symbol resolution** — `gb.symbol(name)` raises on banked ambiguity; use `gb.symbol_in_bank(name, bank)`.
7. **Where to look next** — one-line pointers into `references/` and `assets/recipes/`.

The body must stay under 500 lines per the agentskills.io progressive-disclosure guidance. Detailed surface dumps (every `gb.*` method signature, every error variant, every CLI flag) live in `references/` and are loaded only when needed.

#### 3.7.4 Wiring

The skill is referenced from `CLAUDE.md` alongside the other `.agents/skills` entries already listed there (`qat-bead-closure`, `asm-bead-closure`, `model-contract-bead-closure`, `sequence-state-bead-closure`, `fixture-bead-closure`, `logging-bead-closure`):

```markdown
- For F-A8 (`gbf-debug`) usage — driving the CLI, writing scripts, reading envelopes — use `.agents/skills/gbf-debug-usage/SKILL.md`.
```

The CLAUDE.md edit lands in the same PR as F-A8.

#### 3.7.5 Acceptance criteria (T-A8.8a)

```bash
# Frontmatter and structural validation per the agentskills.io reference library.
skills-ref validate .agents/skills/gbf-debug-usage

# Skill body fits the progressive-disclosure budget.
test "$(wc -l < .agents/skills/gbf-debug-usage/SKILL.md)" -le 500

# Every recipe under assets/ runs end-to-end against the F-A8 tiny-ROM fixture.
cargo test -p gbf-debug -- skill::recipes_run_against_tiny_rom

# CLAUDE.md picks up the new entry.
grep -q "gbf-debug-usage" CLAUDE.md
```

T-A8.8a (`bd-1aaz`) is gated on T-A8.5 (`bd-7fft`, the stateless CLI) so the skill is not authored against a moving target. It does not block the rest of F-A8 — the skill ships in the same PR but is the last commit in the §9.1 linearization. T-A8.8b (`bd-2j4m`, the runtime-ASM conformance smoke suite) is a follow-up bead and is **not** in the F-A8 closing PR; see §11.9.

## 4. Cross-cutting concerns

### 4.1 Determinism contract

Every successful CLI invocation that does not hit a wall-clock watchdog is a pure function of:

- the input session bytes (for `exec` / `inspect`);
- the input ROM bytes + the input `.sym` bytes (for `init`);
- the script bytes (for `exec`);
- the output path string as emitted in the envelope;
- the semantic CLI arguments (`--timeout` if it influences the post-success envelope is excluded, but `--trace-capacity` and any flag that shapes the session is included);
- the `gbf-debug` binary version (changes are an explicit RFC bump and re-snapshot).

The session field `created_at_micros_since_init` is virtual time and starts at `0` for `init`; it is not seeded from wall-clock time and is not seeded from `SOURCE_DATE_EPOCH`. It increments only via the deterministic virtual clock that drives `Date.now()` (§3.2.4).

Wall-clock watchdog failures are liveness failures, not deterministic semantic outcomes; they do not write a normal output session and surface as `ErrorEnvelope { kind: "watchdog_timeout", ... }`. With `--write-partial-on-timeout` the CLI writes a session marked `determinism: "nondeterministic_partial"` for forensic inspection only — never as input to the determinism golden test.

The deterministic-output golden test pins the SHA-256 of a checked-in fixture session after one `init` round and one `exec` round, plus the SHA-256 of the canonical stdout envelope (omitting `metrics`). Any change to the JSON serialization order, the canonicalization rules, the zstd compression level, or the JS host setup that perturbs the output bytes will fail the golden test.

The list of determinism guards (cumulative across F-A7 and F-A8):

- `gbf-emu::DeterminismPolicy::default()` — fixed RTC, RNG seed, audio off (F-A7 territory).
- `BTreeMap` and `Vec` (sorted at construction time for symbol entries) for every collection in the on-disk schema.
- Recursive canonicalization of every `serde_json::Value` produced from JS before envelope serialization (object keys sorted lexicographically). This guard is independent of `serde_json`'s `preserve_order` feature; it always runs.
- Derived struct serialization emits fields in declaration order, and `BTreeMap` emits entries in key order.
- zstd level fixed at `3`.
- `Date.now()` is virtual time (driven by the emulator), not host elapsed time. `Math.random()` is fixed-seeded `xorshift64*`. `console` is deleted.
- `created_at_micros_since_init` is virtual time, not seeded from wall-clock or `SOURCE_DATE_EPOCH`.
- Default envelopes contain no host-duration fields. `ExecMetrics` is opt-in via `--emit-metrics` and excluded from the golden test.

### 4.2 Atomic write protocol

`Session::write_new(path)` writes to a temporary file in the **same directory** as `path` (so the rename is atomic on POSIX), calls `File::sync_all()`, renames into place, and (where the OS supports it) fsyncs the parent directory. By default `write_new` rejects an existing `path` so a second invocation does not silently clobber a sibling session.

`Session::replace(path)` is used only when the CLI flag `--replace-existing-out` is present. Replacement semantics are implemented through the same cross-platform atomic-write helper used by F-A6, or through a documented platform-specific replacement path (e.g. `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING` on Windows). F-A8 does not rely on bare `std::fs::rename(tmp, path)` as a cross-platform replacement primitive, because rename-over-existing semantics differ across OSes.

Properties enforced by both helpers:

- A torn write never produces a half-`.gbsess` at the canonical path.
- The `<random>` suffix on the tempfile avoids collision if multiple `gbf-debug` processes target the same parent directory.
- F-A8 does *not* take a file lock. Sessions are user-owned; concurrency is the user's problem. Two `gbf-debug exec --out same.gbsess --replace-existing-out` invocations racing each other will produce one of the two outputs nondeterministically; F-A8 documents this in the CLI help text but does not coordinate.

`gbf-debug exec --in same.gbsess --out same.gbsess` is **rejected** at argument-parse time with `CliError::InOutSamePath` (exit code 1). In-place mutation destroys the parent/child session model and interacts badly with crash recovery; users who want a fork overwrite must write a new output path and `mv` it themselves.

### 4.3 The `gbf-emu` adapter shape

F-A8 imports the following symbols from `gbf-emu`. **These are the F-A7 public-API names as of `c269c4f` (the F-A7 merge commit), not placeholders.** If a future `gbf-emu` revision renames any of them, the F-A8 PR rebases on the new names rather than papering over the divergence in adapter shims.

**Construction and restore.**

- `Emulator`, `EmulatorBuilder`, `EmulatorConfig { policy, boot_mode, trace_capacity, trace_drop_policy, audit_host_pokes }`, `BootMode::PostBootDmg` (the default), `DeterminismPolicy::default()`.
- `Emulator::load_rom(bytes: &[u8], config: EmulatorConfig) -> Result<Self, EmuError>`.
- `Emulator::restore(&mut self, snapshot: &Snapshot) -> Result<(), EmuError>` — re-applies machine state in place and validates `SnapshotLineage`. Note this **mutates `self`** and returns `()`, not a fresh `Emulator`.
- `Emulator::snapshot(&self) -> Result<Snapshot, EmuError>` — produces a `Snapshot { blob, lineage, trace_bank }` that already excludes F-A8 debug-dispatcher state by construction (the dispatcher state lives in `Emulator::traps()` outside the snapshotted gameroy save-state, not inside it).
- `Snapshot` derives `Serialize`/`Deserialize`; F-A8 serializes it directly through serde with no `to_bytes`/`from_bytes` helper.

**Cycle units.**

- `ClockCycles(pub u64)`, `MCycles(pub u64)`, `CycleBudget` (variants `Clock(ClockCycles)` and `Machine(MCycles)`); `CycleBudget::as_clock_cycles(self) -> ClockCycles`.

**Memory accessors on `Emulator`.**

- `peek(&self, addr: u16) -> Result<u8, EmuError>` — single-byte side-effect-free read.
- `peek_range(&self, start: u16, len: usize) -> Result<Vec<u8>, EmuError>` — side-effect-free range read. Note `len: usize`, not `u32`; F-A8's JS-side `gb.read(addr, len)` casts after the `addr+len <= 0x10000` overflow check.
- `poke(&mut self, addr: u16, value: u8) -> Result<(), EmuError>` — side-effect-free debugger write (single byte).
- `bus_read(&mut self, addr: u16) -> Result<u8, EmuError>` — side-effecting CPU-bus read; records `TraceOrigin::HostBus`.
- `bus_write(&mut self, addr: u16, value: u8) -> Result<(), EmuError>` — side-effecting CPU-bus write; records `TraceOrigin::HostBus`.

**Execution.**

- `Emulator::step(&mut self) -> Result<StepOutcome, EmuError>` — single-instruction step. F-A8's JS-visible `gb.step(n)` is a host-side loop over this; there is no `step(n)` on the F-A7 surface.
- `Emulator::run_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError>`.
- `Emulator::run_fast_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError>` (additional perf-optimized path; F-A8 M0 does not use it).
- `Emulator::run_frame(&mut self) -> Result<RunOutcome, EmuError>` and `run_fast_frame` (frame-granularity helpers; F-A8 M0 does not use them).
- `Emulator::run_until_pc(&mut self, pc: u16, budget: CycleBudget) -> Result<RunOutcome, EmuError>`.
- `RunOutcome::BudgetElapsed { observed: ClockCycles, requested: ClockCycles }` | `TrapHit { trap_id: BreakpointId, kind: TrapKind, observed: ClockCycles }` | `Idle { state: CpuIdleState, observed: ClockCycles }`. There is no `TimeBudgetExpired` variant.

**Register and IO accessors.**

- `Emulator::regs(&self) -> Regs` — `Regs` is F-A7's flat POD with `pub a, b, c, d, e, h, l, f, sp, pc, ime: ImeSnapshot`. The InspectEnvelope's `RegsSnapshot` (§3.5.4) is F-A8-defined and is built by decoding `Regs`.
- `ImeSnapshot::{Disabled, Enabled, ToBeEnable}` — the JS-side `ime` string mirrors these as `"disabled" | "enabled" | "to_be_enable"`.
- `Emulator::set_regs(&mut self, regs: Regs) -> Result<(), EmuError>` — F-A8 does not expose register writes through JS in M0 (see §3.3.2).
- `Emulator::framebuffer(&mut self) -> Framebuffer` — `Framebuffer` is the typed wrapper with `pixel(x, y) -> u8`. F-A8's JS `gb.framebuffer()` flattens the typed framebuffer into a 23040-byte `Uint8Array` row-major.
- `Emulator::set_joypad(&mut self, frame: JoypadFrame)` — note `frame: JoypadFrame`, not `JoypadState`. `JoypadFrame::pressed(button)` and `with(button)` are the constructors; `is_pressed(button)` is the accessor.

**Trap dispatcher.** F-A7 ships an `add`-style API, not the originally-sketched `register`-style API.

- `Emulator::traps(&mut self) -> &mut TrapDispatcher` — the only way F-A8 gets a handle on the dispatcher.
- `TrapDispatcher::add_pc(&mut self, addr: u16, predicate: Predicate, action: TrapAction) -> BreakpointId`.
- `TrapDispatcher::add_mem_read(&mut self, range: AddressRange, predicate, action) -> BreakpointId`.
- `TrapDispatcher::add_mem_write(&mut self, range: AddressRange, predicate, action) -> BreakpointId`.
- `TrapDispatcher::add_mem_rw(&mut self, range: AddressRange, predicate, action) -> BreakpointId`.
- `TrapDispatcher::remove(&mut self, id: BreakpointId) -> bool` (or `remove_entry(id) -> Option<RemovedTrap>` to recover the removed kind/predicate).
- `TrapDispatcher::list()`, `is_empty()`, `has_pc_traps()`, `has_memory_traps()`, `clear()`.
- `TrapDispatcher::export_persistable_specs(&self) -> Result<Vec<TrapSpec>, TrapPersistenceError>` — F-A8 does not use this for its own session persistence (F-A8 owns its own `BreakpointPersisted` schema), but the existence of the method is why F-A8's stringified-source semantics align cleanly with F-A7's `Predicate::Source(_)` persistence shape.

**Trap kinds.** Parameterized variants, not unit variants.

- `TrapKind::Pc { addr: u16 }`.
- `TrapKind::MemRead { range: AddressRange }`.
- `TrapKind::MemWrite { range: AddressRange }`.
- `TrapKind::MemRw { range: AddressRange }` — note `MemRw`, not `MemReadWrite`.
- `AddressRange::new(start: u16, end_inclusive: u16) -> Result<AddressRange, AddressRangeError>` — single-byte ranges are valid (`start == end_inclusive`).

**Predicate and action.**

- `Predicate::Always` — F-A8 uses this for unconditional persisted breakpoints.
- `Predicate::Closure(Box<TrapPredicate>)` where `TrapPredicate = dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static`. F-A8 wraps both JS-closure-shaped *and* JS-string-shaped predicates as Rust closures registered through this variant; F-A7's dispatcher returns `TrapPredicateError::SourceRequiresEvaluator` if asked to evaluate `Predicate::Source(_)` directly, so F-A8 does not register `Source` itself.
- `Predicate::Source(String)` — exists on the F-A7 surface for persistence-friendly export, not for direct evaluation. F-A8 keeps the source string in its own `BreakpointPersisted` schema.
- `TrapAction::{HaltAndReport, Continue}` — F-A8 always uses `HaltAndReport` (a `gb.add_breakpoint` call always wants the run loop to stop and surface the hit through `RunStopReason::Breakpoint`).

**Trap context.**

- `TrapContext<'a> { regs: Regs, pc: u16, access: Option<MemoryAccess>, cycle: ClockCycles, view: EmuReadOnlyView<'a> }`. `EmuReadOnlyView` exposes `peek(addr)` / `peek_range(start, len)`. F-A8's stringified-predicate scope is `{ regs, pc, access, cycle, view, symbol(name), symbolInBank(name, bank) }` — `symbol` and `symbolInBank` are F-A8 additions for ergonomic symbol resolution inside predicates and resolve through the in-session `SessionSymbolTable`.
- `MemoryAccessKind::{InstrFetch, DataRead, Write, Push, Pop, ...}` — opaque to F-A8; the JS-side predicate scope re-exposes the access shape verbatim.

**Trace.**

- `NormalizedTraceEvent` and `TraceOrigin::{GuestCpu, Dma, HostBus, HostPoke}` — the canonical event shape consumed by F-A8's `TraceEventPersisted` boundary converter (§3.1.7).
- `TraceCursor`, `TraceDropPolicy::{DropOldest, DropNewest}`, `BankSnapshot`, `BankSwitchSource`, `TraceMapper` — exposed for completeness; F-A8 stores `BankSnapshot` inside `Snapshot::trace_bank` automatically as part of `Emulator::snapshot()`.
- `Emulator::drain_trace(&mut self) -> Vec<NormalizedTraceEvent>` — F-A8's `gb.trace_ring()` reads from this drain plus the F-A8-side `TraceRing` accumulator.

**Errors.**

- `EmuError` — note the name is `EmuError` (not `EmulatorError`). F-A8's `CliError::EmulatorRestore`, `CliError::Io`, etc., wrap it where appropriate.
- `EmuVersionTag::current()` — pinned in `Snapshot::lineage.emu_version`; F-A8 includes the tag in the `InspectEnvelope` metadata so a session whose lineage came from a different `gbf-emu` build is visible to the operator.

### 4.4 The `gbf-asm` adapter shape

F-A8 imports exactly one item: `gbf_asm::symbols::parse_sym_entries`. F-A8 does not depend on `Builder`, `Section`, `Instr`, `LayoutPlan`, or any of the eDSL machinery — the ROM is consumed as opaque bytes from disk.

### 4.5 `no_std`?

`gbf-debug` is not `no_std` and does not aspire to be. The CLI has filesystem I/O at every entry point, the JS host is `std`-bound, and the binary target needs `std`. The "`no_std + alloc` capable" engineering rule applies to `gbf-foundation`, `gbf-artifact`, `gbf-abi`, `gbf-ir`, and `gbf-asm`; `gbf-debug` is explicitly outside that list (alongside `gbf-store`, `gbf-codegen`, `gbf-emu`).

### 4.6 Error handling style

Every fallible API returns a typed error:

```rust
#[derive(Debug)]
pub enum CliError {
    Io(std::io::Error),                                       // not Clone
    SessionLoad(SessionLoadError),
    SessionWrite(SessionWriteError),
    SymbolHydration(SymbolHydrationError),
    ScriptError(ScriptError),
    PredicateCompileFailed { addr: u16, source: String, error: String },
    PostLoadPcUnexpected { observed: u16 },                   // F-A7 contract drift
    CliArgs { message: String },                              // mapped from clap::Error
}
```

Error → `ErrorEnvelope` translation lives in `cli::emit_error_envelope`. The exit-code mapping is centralized in the binary main (see §3.4.5).

`std::io::Error` wraps the underlying I/O failure verbatim; in the envelope it surfaces as `{"kind": "io", "message": "<Display of io::Error>"}`.

## 5. Errors

Every fallible API in `gbf-debug` returns one of:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionLoadError {
    BadMagic { observed: [u8; 4], expected: [u8; 4] },
    BadFlags { observed: u32 },
    Truncated { observed: usize, minimum: usize },
    ZstdDecode(String),
    JsonDecode(String),
    SchemaMismatch { observed: u32, current: u32 },
    RomHashMismatch { observed: [u8; 32], expected: [u8; 32] },
    SnapshotRomMismatch { snapshot_rom_sha256: [u8; 32], session_rom_sha256: [u8; 32] },
    UnsupportedBootMode { observed: String },     // F-A8 M0 sessions are PostBootDmg-only
}

#[derive(Debug)]
pub enum SessionWriteError {
    Io(std::io::Error),                          // not Clone (std::io::Error is not Clone)
    JsonEncode(String),
    ZstdEncode(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolHydrationError {
    SymParse(String),                            // wraps gbf_asm::symbols::SymError
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    SyntaxError { message: String, line: u32, column: u32 },
    RuntimeException { message: String, line: Option<u32>, column: Option<u32>, function: Option<String> },
    Timeout { elapsed_micros: u64 },
    OutOfMemory,
    StackOverflow,
    HostBindingError { method: String, source: String },
}
```

`SymbolHydrationError` is fatal-only (genuinely-malformed `.sym` lines). Duplicate names are non-fatal hydration warnings carried in `SymbolHydration::warnings`; they are not error variants.

Every error implements `core::fmt::Display` and `std::error::Error`. `gbf-debug` is a `std` crate and has no Cargo feature named `std`. `Display` does not allocate beyond what the contained `String`s already cost. Errors that wrap `std::io::Error` are not `Clone`; callers that need to copy them should `format!` and propagate the message.

## 6. Testing strategy

### 6.1 Layers

- **Unit tests** per module: in-file `#[cfg(test)] mod tests` blocks for the type-level invariants.
- **Integration tests** (`tests/`): every public API exercised end-to-end.
- **Golden session fixtures** (`tests/fixtures/`): a handful of small sessions whose SHA-256 is checked into the test, regenerable via `cargo test -- --ignored regenerate_fixtures`.
- **CLI smoke tests** (`tests/cli_smoke.rs`): invoke the library APIs directly (not the binary, to avoid process-spawn cost), assert envelope shapes.
- **End-to-end test** (`tests/e2e.rs`): build a tiny ROM via `gbf-asm::rom`, `init` a session, `exec` a small script, `inspect` the result. Asserts the full pipeline.
- **Determinism test** (`tests/determinism.rs`): run the same `init` and `exec` twice, assert byte-equal output sessions and byte-equal stdout envelopes.
- **Schema-evolution negative tests**: hand-crafted bytes with `schema_version = 0` and `schema_version = 999` both fail `Session::load_bytes` with `SchemaMismatch`.

### 6.2 Test ROM

The end-to-end test relies on a tiny ROM produced by `gbf-asm::rom`. The ROM:

- has a valid cartridge header (Nintendo logo, MBC type, ROM size, checksum);
- jumps from `$0100` to a small loop at `$0150` that increments register `A` and halts at a labeled address `done`;
- emits a `.sym` file with one labeled entry (`done` at `$015A` or wherever the loop ends).

The script the e2e test runs:

```js
gb.add_breakpoint(gb.symbol("done"));
gb.run_until_breakpoint();
result = { pc: gb.regs.pc, a: gb.regs.a };
```

The expected envelope `result` is `{ pc: 0x015A, a: <whatever the loop produced> }`. The exact `a` value is pinned in the test.

### 6.3 Property tests

Two property tests:

```rust
#[test]
fn session_round_trip_byte_stable() {
    proptest!(|(s: Session)| {
        let bytes = s.to_bytes()?;
        let s2 = Session::load_bytes(&bytes)?;
        prop_assert_eq!(s, s2);
        prop_assert_eq!(bytes, s2.to_bytes()?);
    });
}

#[test]
fn trace_ring_capacity_invariant() {
    proptest!(|(events: Vec<TraceEventPersisted>, cap: u32)| {
        let mut ring = TraceRing::new(cap.max(1));
        for e in events.iter() { ring.push(e.clone()); }
        prop_assert!(ring.events.len() as u32 <= ring.capacity);
    });
}
```

### 6.4 Determinism

A single `cargo test` run is deterministic given the proptest seed. The crate has no `SystemTime`, `Instant::now` (other than the script timeout deadline, which is a host-side concern not visible in any output bytes), `rand`, or `thread_rng` use anywhere in production paths. The proptest seed is set via env or file in `tests/proptest-regressions/`.

## 7. Dependencies

### 7.1 New direct dependencies

| Dependency       | Purpose                                | Where used | Feature flags |
|------------------|----------------------------------------|------------|---------------|
| `rquickjs`       | embeddable JS engine (QuickJS-NG bindings) | `script.rs` | `default` |
| `zstd`           | session compression                     | `session.rs` | `default` |
| `clap`           | CLI argument parsing                    | `bin/gbf-debug.rs` | `derive` |
| `serde`          | session + envelope serialization        | every module | `derive` (already in workspace) |
| `serde_json`     | JSON wire format                        | `session.rs`, `cli.rs` | already in workspace |
| `base64`         | base64 of `RomBlob` bytes and the `Snapshot::blob` byte field inside JSON | `session.rs` | `default` |
| `sha2`           | SHA-256 of session bytes and ROM bytes  | `session.rs`, `cli.rs` | already in workspace |
| `proptest`       | property tests                          | dev-dep only | none |

`rquickjs` is the load-bearing new dependency. It pulls in QuickJS-NG via a build script; the build is hermetic on x86_64-darwin, x86_64-linux, and aarch64-darwin (the workspace's CI triples). If QuickJS-NG fails to build on a target, the F-A8 implementation reports it; F-A8 does not ship a fallback engine.

The implementation must include `cargo tree -e features -p gbf-debug` in the review packet and must justify every enabled rquickjs feature. In M0, module loading is not part of the JS surface (the `gb` object is a global), so the `loader` feature is disabled unless an implementation test proves it is required by the binding macros. The `rquickjs` allocator features (custom allocator, no-default-allocator) are not enabled, because they change `set_memory_limit` semantics per upstream docs and would invalidate the §3.2.3 memory-limit contract.

### 7.2 Cargo.toml (new file)

```toml
[package]
name = "gbf-debug"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
publish = false

[[bin]]
name = "gbf-debug"
path = "src/bin/gbf-debug.rs"

[dependencies]
gbf-asm        = { path = "../gbf-asm" }
gbf-emu        = { path = "../gbf-emu" }
gbf-foundation = { path = "../gbf-foundation" }
gbf-abi        = { path = "../gbf-abi" }
serde          = { workspace = true, features = ["derive"] }
serde_json     = { workspace = true }
# rquickjs version is the latest tested patch at implementation time; bump only via explicit RFC.
# Default features disabled to keep the QuickJS feature surface auditable; only `std` and `macro`
# are enabled in M0. Module loading (`loader`) is intentionally OFF — F-A8 scripts do not import
# modules; the `gb` object is a global. The implementation must justify any added feature in §7.1.
rquickjs       = { version = "=<fully-specified-tested-patch>", default-features = false, features = ["std", "macro"] }
zstd           = { version = "=<fully-specified-tested-patch>", default-features = false }
base64         = { version = "=<fully-specified-tested-patch>" }
sha2           = { version = "=<fully-specified-tested-patch>" }
clap           = { version = "=<fully-specified-tested-patch>", features = ["derive"] }

[dev-dependencies]
proptest       = { version = "=<fully-specified-tested-patch>" }
```

Per Cargo's specifying-dependencies guidance, exact-version requirements use a fully specified `=major.minor.patch` (e.g. `=1.2.3`). The `<fully-specified-tested-patch>` placeholders above are filled in at implementation time after the engineer pins the actual patch they tested against; the F-A8 PR may not land with these placeholders intact. The Cargo.lock is checked in per the workspace policy.

Workspace `Cargo.toml`'s `[workspace] members = [...]` list gains `"gbf-debug"`.

### 7.3 Why rquickjs and not deno_core / boa / v8

- **rquickjs** (~1 MB of upstream QuickJS-NG source compiled in): no async runtime needed, ergonomic Rust binding via `class!` macro, MIT-licensed, tracks ES2023, well-maintained. Memory limit and interrupt handler are first-class. Best fit.
- **boa** (pure Rust): would avoid the C build step but is significantly slower and has a smaller standard-library coverage. Unattractive for the agent's "loop 10⁶ times" use case.
- **deno_core** (V8-based): pulls in V8 (~50 MB build artifact, complex build), tokio-based async, optimized for server workloads. Massive overkill.
- **v8** (raw): same V8 build problem; no high-level Rust binding.

The choice is rquickjs; the F-A8 implementer should not re-litigate it without a concrete benchmark showing rquickjs cannot meet the target.

## 8. Constitutional grounding

| Constitution clause | F-A8 gate |
|---------------------|-----------|
| §I.1 (correctness by construction) | Typed `PersistedPredicate`, `WatchpointKind`, `TraceEventKind`, `RunStopReason`, `SessionLoadError`. |
| §III (shifting left) | Schema mismatch caught at session-load time; predicate syntax errors caught at registration time, not at trap-fire time. |
| §IV.3 (reproducible builds) | Determinism golden test pins the SHA-256 of a fixture session after one `init` and one `exec`. |
| §V.1 (structured logs only) | Every CLI command emits exactly one JSON envelope; no human-prose CLI output. |
| §V.3 (silence on success, loud on failure) | Success exit `0` with envelope on stdout; failure non-zero with `ErrorEnvelope` on stderr. |
| §VI.1 (single source of truth) | `.sym` parsing routed through `gbf-asm::symbols::parse_sym_entries`; `NormalizedTraceEvent` shape owned by `gbf-emu`. |
| §I.2 (`unsafe` forbidden) | `#![forbid(unsafe_code)]` in every `.rs` file. |

## 9. Tasks and ordering

### 9.1 Task graph

```
T-A8.1 (session schema)
   │
   ├── T-A8.6 (symbol embedding)         ← depends on session schema for SessionSymbolTable shape
   ├── T-A8.7 (predicate persistence)    ← depends on session schema for PersistedPredicate shape
   │
T-A8.2 (rquickjs host)
   │
   ├── T-A8.3 (gb object binding)        ← depends on T-A8.1 (BreakpointPersisted) + T-A8.2 (host) + F-A7 (Emulator/TrapDispatcher)
   │
T-A8.4 (structured CLI output)           ← depends on T-A8.3 (the script outcome types)
T-A8.5 (stateless CLI)                   ← depends on T-A8.1 + T-A8.3 + T-A8.4 + F-A1 (parse_sym_entries) + F-A7 (Emulator)
   │
   ├── T-A8.8a (agent skill SKILL.md)    ← depends on T-A8.5 (CLI surface frozen); authored against the same surface the recipes exercise
   └── T-A8.8b (runtime-ASM conformance) ← depends on T-A8.5 + T-A8.6; can land in a follow-up PR
```

Critical path: T-A8.1 → T-A8.2 → T-A8.3 → T-A8.5 → T-A8.8a. T-A8.6, T-A8.7 parallelize after T-A8.1. T-A8.4 parallelizes after T-A8.3. T-A8.8a (the agent skill) is the last commit in the F-A8 PR. T-A8.8b (the scripted runtime-ASM conformance smoke suite, `bd-2j4m`) is **out of the F-A8 closing PR** — it bridges F-A1/F-A4/F-A5 emitted-ROM behavior to the F-A8 debugger surface, and the bead's own description treats it as a follow-up gate that runs once F-A4/F-A5 have ROM fixtures to point at. Treat it as the first deliverable in a "M0 conformance" follow-up PR, not as an F-A8 blocker.

### 9.2 PR shape

**The F-A8 closing PR contains tasks T-A8.1 through T-A8.8a.** All eight of those (the seven core tasks plus the agent skill) land together. Total ~1,800–2,200 LOC of production code + ~2,000 LOC of tests + the agent skill (~500 lines of `SKILL.md` plus references and recipes) + the F-A8 review packet (§14). T-A8.8b (runtime-ASM conformance smoke suite) lands in a separate follow-up PR scoped against `bd-2j4m`; that PR is gated on T-A8.8a closing plus F-A4/F-A5 having committed ROM fixtures.

Three reasons the crate must land atomically rather than across stacked PRs:

1. **The CLI is the deliverable.** Without `init` + `exec` + `inspect` all working, there is no way to validate any of the lower layers end-to-end. A partial PR (say, "session schema only") ships type definitions but no exercised path; the next reviewer would have to take the type shapes on faith.
2. **The session schema constrains every other slice.** Splitting the schema PR from the JS-binding PR forces an interim "stub PR with zero JS coverage" whose only job is to lock the schema before the binding lands. That ordering is a file-write ordering, not a PR boundary; the §9.1 linearization handles it as commit ordering inside one PR.
3. **No external consumer exists.** F-A8's only consumer is the agent invoking the binary. There is no Rust-code dependent demanding partial delivery; the only thing a multi-PR split would buy is review-overhead amortization, which the single review packet (§14) already addresses.

The single PR ships with the full F-A8 review packet (§14). The §9.1 task-graph linearization (T-A8.1 → T-A8.6 + T-A8.7 in parallel → T-A8.2 → T-A8.3 → T-A8.4 → T-A8.5 → T-A8.8a) is the **commit ordering inside this single PR**, not a PR sequence. Engineers picking up F-A8 should land commits in that order so each commit individually compiles and the tests introduced by that commit pass. T-A8.8a (the agent skill) is the last commit because the skill's recipes are validated against the CLI surface frozen by T-A8.5; landing the skill earlier would require revising it as the surface settles. The full F-A8 end-to-end suite (including `skills-ref validate` and the recipe end-to-end run) is required at the final commit. The merge into `main` is one PR and one squash (or one merge commit) per the project's normal cadence. T-A8.8b (`bd-2j4m`, the runtime-ASM conformance smoke suite) is a follow-up PR (§11.9), not a commit inside the F-A8 closing PR.

## 10. Risk register

| Risk | Mitigation |
|------|------------|
| rquickjs upstream changes break the `class!` macro between minor versions | Pin to `=0.6.x`; bump only via explicit RFC. |
| QuickJS-NG build fails on a CI triple | F-A8 ships only the CI triples currently in tree (x86_64-{darwin,linux}, aarch64-darwin); a new triple opens a follow-up bead with a build report. |
| Wall-clock timeout fires too late on a tight CPU loop | Default 30 s timeout is generous; the QuickJS interrupt handler runs every N opcodes (~tens of µs of wall clock). If a real workload bumps against this, the operator passes `--timeout`. |
| A script's `Date.now()` accidentally depends on wall-clock time | Replaced with deterministic virtual time driven by emulator M-cycles; tested by `script::date_now_is_virtual_time_not_wallclock` + `script::date_now_reproducible_across_runs`. |
| Closure-shaped predicates silently lose their condition on session write | Persisting drops the closure, but the dropped-predicate warning is emitted in the `ExecEnvelope` so the agent learns at write time. The two predicate kinds being explicit prevents accidental closure persistence. |
| zstd compression level changes between releases shift output bytes | Level pinned to `3`; the determinism golden test fails on any drift. |
| gameroy save-state format changes between gameroy versions | Pin gameroy version at the workspace level (already standard practice for Cargo deps); a gameroy bump requires re-generating fixtures. |
| `serde_json` reorders object keys (e.g., workspace feature unification toggles `preserve_order`) | Derived struct serialization emits fields in declaration order, and `BTreeMap` emits entries in key order. For `serde_json::Value` objects produced from JS, F-A8 recursively sorts object keys before serialization (§3.5.2), so envelope output is byte-stable regardless of the active `serde_json` features. |
| Two concurrent `gbf-debug exec --out same.gbsess` invocations race | F-A8 documents the race in CLI help; does not coordinate. The atomic-rename protocol means at least one of the two writes succeeds with consistent contents; the other gets overwritten. |
| Snapshot map grows unbounded in a long script | Cap at 32 snapshots per invocation (configurable); exceeding is `HostBindingError::SnapshotLimitExceeded`. |
| The `.gbsess` file grows unbounded as the trace ring fills | `TraceRing::capacity` caps event count; the `dropped` counter records overflow. Default capacity 1024 events ≈ 50–100 KB of session bloat. |
| `parse_sym_entries` rejects a real `.sym` file because of a non-spec line | F-A1's parser is the source of truth; if `gbf-asm` produces a line F-A8 cannot parse, that is a `gbf-asm` bug, not an F-A8 bug. F-A8's `SymbolHydrationError::SymParse` carries the underlying error verbatim. |

## 11. Tasks T-A8.1..T-A8.8b (deep-dive cross-reference)

Every child task in beads has a one-paragraph entry below mapping to the §3 design.

### 11.1 T-A8.1 (`bd-1ckj`) — Session file format

Owns: `Session`, `BreakpointPersisted`, `WatchpointPersisted`, `WatchpointKind`, `TraceRing`, `TraceEventPersisted`, `TraceEventKind`, `RomBlob`, `EmulatorSnapshotBlob`, `SessionMetadata`, `SCHEMA_VERSION`, `SessionLoadError`, `SessionWriteError`. See §3.1.

Acceptance gates:

- `cargo test -p gbf-debug -- session::magic_round_trip`
- `cargo test -p gbf-debug -- session::flags_must_be_zero`
- `cargo test -p gbf-debug -- session::schema_version_pinned`
- `cargo test -p gbf-debug -- session::schema_mismatch_is_fatal`
- `cargo test -p gbf-debug -- session::serde_round_trip`
- `cargo test -p gbf-debug -- session::write_then_read_byte_identical`
- `cargo test -p gbf-debug -- session::trace_ring_capped`
- `cargo test -p gbf-debug -- session::breakpoint_predicate_round_trip`
- `cargo test -p gbf-debug -- session::watchpoint_kinds_exhaustive`

### 11.2 T-A8.2 (`bd-2ulg`) — rquickjs scripting host

Owns: `ScriptHost`, `ScriptConfig`, `ScriptError`, `ScriptOutcome`, the wall-clock-deadline interrupt handler, the `Date.now`/`Math.random`/`console` deterministic stubs. See §3.2.

Acceptance gates:

- `cargo test -p gbf-debug -- script::evaluate_returns_result`
- `cargo test -p gbf-debug -- script::syntax_error_carries_position`
- `cargo test -p gbf-debug -- script::runtime_exception_carries_position`
- `cargo test -p gbf-debug -- script::timeout_aborts`
- `cargo test -p gbf-debug -- script::date_now_is_virtual_time_not_wallclock`
- `cargo test -p gbf-debug -- script::math_random_is_seeded`
- `cargo test -p gbf-debug -- script::console_is_deleted`
- `cargo test -p gbf-debug -- script::memory_limit_enforced`

### 11.3 T-A8.3 (`bd-3psj`) — gb object binding

Owns: the `gb` JS class binding plus the parallel closure-predicate map. See §3.3.

Acceptance gates: see §3.3.10.

### 11.4 T-A8.4 (`bd-3shw`) — Structured CLI output

Owns: `ExecEnvelope`, `InitEnvelope`, `InspectEnvelope`, `ErrorEnvelope`, `LogEntry`, `Warning`, `RegsSnapshot`, `TraceRingSummary`, `SymbolsSummary`, `ExecMetrics`. See §3.5.

Acceptance gates: see §3.5.6.

### 11.5 T-A8.5 (`bd-7fft`) — Stateless CLI

Owns: `run_init`, `run_exec`, `run_inspect`, `InitArgs`/`ExecArgs`/`InspectArgs`, the `bin/gbf-debug.rs` clap-driven dispatcher, the exit-code mapping, the atomic-write protocol. See §3.4.

Acceptance gates: see §3.4.6.

### 11.6 T-A8.6 (`bd-2i1i`) — Symbol embedding & rehydration

Owns: `SessionSymbolTable`, `SessionSymbolEntry`, `SymbolHydrationError`, the `from_sym_text` integration with `gbf_asm::symbols::parse_sym_entries`, the `gb.symbol(name)` and `gb.symbol_at(addr)` JS bindings. See §3.1.4 + the symbol surface portion of §3.3.

Acceptance gates: see §3.1.9.

### 11.7 T-A8.7 (`bd-24ju`) — Predicate persistence

Owns: `PersistedPredicate`, the parallel closure-predicate map inside the host, the persistence policy (closure → `None` + warning, string → `StringifiedSource`). See §3.1.5 + the breakpoint surface portion of §3.3.

Acceptance gates: see §3.1.10.

### 11.8 T-A8.8a (`bd-1aaz`) — Agent skill `SKILL.md`

Owns: `.agents/skills/gbf-debug-usage/SKILL.md` (frontmatter + body), `references/{ENVELOPE,GB_OBJECT,PREDICATES,DETERMINISM,RECIPES}.md`, `assets/recipes/*.js`, the CLAUDE.md entry that points at the new skill, and the per-recipe end-to-end test that runs each recipe under `gbf-debug exec` against the F-A8 tiny-ROM fixture. See §3.7.

Depends on T-A8.5 — the skill is authored against the frozen CLI surface so the recipes do not drift while the binding is still moving. T-A8.8a lands as the last commit in the §9.1 linearization for the F-A8 closing PR. (Originally numbered T-A8.8 in the bead title; renamed T-A8.8a here once `bd-2j4m` was filed as a sibling task that also carries the T-A8.8 prefix in beads.)

Acceptance gates: see §3.7.5.

### 11.9 T-A8.8b (`bd-2j4m`) — Scripted runtime-ASM conformance smoke suite

Owns: an integration suite (whose home crate — `gbf-test`, `gbf-debug`, or a new `gbf-conformance` — is the implementation owner's call) that drives the canonical `gbf-emu` + `gbf-debug` path against richer Epic-A ROM fixtures than `tiny_rom`. Bridges F-A1's static `tiny_rom` artifact checks, F-A7's emulator boot smoke, and F-A8's scriptable debugger surface so emitted F-A4/F-A5 assembly is verified to behave correctly when executed as a real Game Boy ROM.

Depends on T-A8.5 (CLI surface frozen) **and** T-A8.6 (symbol embedding so scripts can refer to F-A4/F-A5 labels by name) **and** F-A4/F-A5 producing committed ROM fixtures. The bead is **not** in the F-A8 closing PR — it is the first deliverable in a follow-up "M0 conformance" PR. F-A8 ships the surface; T-A8.8b consumes it.

Acceptance gates (paraphrased from `bd-2j4m`):

- A `gbf-debug exec` script suite that boots each committed F-A4/F-A5 ROM fixture, runs to a labeled checkpoint, and asserts on register/memory state plus expected `NormalizedTraceEvent` sequences.
- A reproducibility manifest (path layout the implementation owner's call) tying each ROM fixture to its source assembly, its `.sym` file, the script that drives it, and the expected envelope fingerprint.
- The suite is fast and deterministic and runs as part of the M0 gate before "agent-debuggable from the first ROM that boots" can be claimed.

## 12. Claim-to-gate matrix (closure-style)

| Claim | Gating test / artifact |
|-------|------------------------|
| `.gbsess` magic is `b"GBSE"` | `session::magic_round_trip` |
| FLAGS field rejects non-zero | `session::flags_must_be_zero` |
| `SCHEMA_VERSION = 1` is pinned | `session::schema_version_pinned` |
| `Session::load` on schema mismatch returns `SchemaMismatch` (no auto-migrate) | `session::schema_mismatch_is_fatal` |
| `Session` round-trips through serde+zstd byte-identically | `session::write_then_read_byte_identical` + `session_round_trip_byte_stable` (proptest) |
| `TraceRing` honors its capacity | `session::trace_ring_capped` + `trace_ring_capacity_invariant` (proptest) |
| `BreakpointPersisted::predicate` round-trips through serde for both variants | `session::breakpoint_predicate_round_trip` |
| `WatchpointKind::ALL` covers Read, Write, ReadWrite | `session::watchpoint_kinds_exhaustive` |
| `parse_sym_entries` integration produces a deterministic, sorted `SessionSymbolTable` | `symbols::from_sym_text_round_trip` + `symbols::sorted_canonical` |
| Duplicate symbol names are non-fatal warnings; unqualified resolution is ambiguous when multiple banks match | `symbols::duplicate_name_warned_not_fatal` + `symbols::unqualified_duplicate_is_ambiguous` + `symbols::resolve_in_bank_disambiguates` |
| `gb.symbol_at(addr)` returns the symbol name | `symbols::resolve_at_returns_name` |
| Closure-shaped predicates are invocation-local and never become unconditional persisted breakpoints | `predicate::closure_is_invocation_local` + `predicate::closure_does_not_create_unconditional_persisted_breakpoint` + `predicate::closure_drop_is_warned` |
| String predicates evaluate in a read-only `{regs, trap, symbol}` scope | `predicate::stringified_predicate_scope_is_read_only` + `predicate::stringified_predicate_cannot_mutate_emulator` |
| Stringified-source predicates round-trip across `exec` | `predicate::stringified_round_trip` + `predicate::stringified_re_evaluated_on_next_exec` |
| `ScriptHost::evaluate` returns the script's `result` global | `script::evaluate_returns_result` |
| Syntax errors carry line/column | `script::syntax_error_carries_position` |
| Runtime exceptions carry line/column | `script::runtime_exception_carries_position` |
| Wall-clock timeout aborts via QuickJS interrupt handler | `script::timeout_aborts` |
| `Date.now()` is virtual time (driven by the emulator), not wall-clock | `script::date_now_is_virtual_time_not_wallclock` + `script::date_now_reproducible_across_runs` |
| `new Date(...)` throws `TypeError` | `script::date_constructor_throws` |
| Only `Math.random` is replaced; other `Math.*` preserved | `script::math_random_is_seeded` + `script::math_other_methods_preserved` |
| Only `globalThis.result` is captured (`let result = ...` is not) | `script::only_globalThis_result_is_captured` |
| `Math.random()` is seeded deterministically | `script::math_random_is_seeded` |
| `console` is unavailable | `script::console_is_deleted` |
| Memory limit is enforced | `script::memory_limit_enforced` |
| `gb.regs` returns a fresh snapshot; writes are rejected; `ime` is tri-state | `gb::regs_round_trip` + `gb::regs_write_rejected` + `gb::regs_ime_is_tri_state_string` |
| `gb.read` is side-effect-free (F-A7 `peek_range`); `gb.bus_read`/`gb.bus_write` are side-effecting and emit `TraceOrigin::HostBus` | `gb::read_is_side_effect_free` + `gb::bus_read_emits_host_bus_trace` + `gb::bus_write_emits_host_bus_trace` |
| `gb.read`/`gb.write` reject `addr+len` overflow | `gb::read_rejects_overflow` + `gb::write_rejects_overflow` |
| `gb.step(n)` returns `{ pc_after, clock_cycles_consumed, m_cycles_floor_consumed }` (cycle fields decimal strings) | `gb::step_returns_pc_and_cycles` + `gb::step_cycle_fields_are_decimal_strings` |
| `gb.step(n)` is bounded by `max_step_instructions_per_call` | `gb::step_n_capped_by_max_step_instructions_per_call` |
| Memory-watchpoint predicates see post-instruction `regs`/`pc` under M0 backend | `gb::watchpoint_predicate_sees_post_instruction_state` |
| `Date.now()` advances with emulator `ClockCycles` via DMG clock rate | `script::date_now_advances_with_emulator_clock_cycles` |
| `gb.run_until(pc)` stops at PC | `gb::run_until_pc_stops` |
| `gb.run_until(pc, max_m_cycles)` caps at the bound | `gb::run_until_max_cycles_caps` |
| `gb.run_until*` without explicit cap uses `ScriptConfig::default_run_budget` (typed `CycleBudget`) | `gb::run_until_without_explicit_cap_uses_default_budget` + `gb::host_call_cannot_bypass_watchdog_budget` |
| `gb.run_until(pc, ...)` does not leak its one-shot trap into the persisted map | `gb::run_until_one_shot_trap_is_not_persisted` |
| `gb.snapshot()`/`gb.restore()` brackets transient branching and excludes debug-dispatcher state | `gb::snapshot_restore_branching` + `gb::snapshot_excludes_debug_traps` |
| Snapshot limit enforced | `gb::snapshot_limit_enforced` |
| `gb.symbol`, `gb.symbol_in_bank`, `gb.symbol_at`, `gb.symbol_at_in_bank` resolve; ambiguous raises | `gb::symbol_resolves` + `gb::symbol_ambiguous_raises` + `gb::symbol_in_bank_disambiguates` + `gb::symbol_at_resolves` + `gb::symbol_at_ambiguous_raises` + `gb::symbol_at_in_bank_disambiguates` |
| `gb.framebuffer()` returns a 23040-byte array | `gb::framebuffer_shape` |
| `gb.input([])` releases all buttons | `gb::input_empty_releases_all_buttons` |
| `gb.input(buttons)` rejects unknown button names | `gb::input_unknown_button_rejected` |
| `gb.trace_ring()` exposes events in head-to-tail order; `seq` is a string in JS | `gb::trace_ring_shape` + `gb::trace_ring_seq_is_string_in_js` |
| `ExecEnvelope` serializes with a stable shape | `output::exec_envelope_serializes` |
| `InitEnvelope` serializes with a stable shape | `output::init_envelope_serializes` |
| `InspectEnvelope` serializes with a stable shape | `output::inspect_envelope_serializes` |
| `ErrorEnvelope` serializes with a stable shape | `output::error_envelope_serializes` |
| `log()` entries are appended in call order | `output::log_entries_ordered` |
| `ts_micros_since_script_start` is monotone non-decreasing | `output::ts_micros_monotone` |
| Non-serializable `result` becomes JSON `null` plus warning | `output::result_not_serializable_warns` |
| `Warning` does not change exit code | `output::warning_does_not_change_exit_code` |
| `init` writes a new session at the requested path | `cli::init_writes_session` + `cli::init_envelope_shape` |
| `init` with no `.sym` uses an empty symbol table | `cli::init_no_sym_uses_empty_table` |
| `exec` runs the script and writes a new session | `cli::exec_runs_script` + `cli::exec_envelope_shape` |
| `exec` persists breakpoints across invocations | `cli::exec_persists_breakpoints` |
| `exec` writes the post-error session even when the script throws | `cli::exec_writes_session_on_script_error` |
| `exec` sets `parent_sha256` to the SHA-256 of the input session bytes | `cli::exec_parent_sha256_matches_input` |
| `inspect` does not run any script and dumps state | `cli::inspect_no_run_state_dump` + `cli::inspect_envelope_shape` |
| `init → exec → inspect` round-trip works against a tiny ROM | `cli::e2e_init_exec_inspect` |
| Two `exec` runs with same inputs produce byte-identical output sessions | `cli::deterministic_two_runs_byte_identical` |
| Default success envelopes contain no host-duration fields | `output::default_envelope_contains_no_host_timing` |
| Wall-clock watchdog failures do not write a normal output session | `cli::watchdog_timeout_does_not_write_deterministic_session` |
| `--write-partial-on-timeout` writes a partial session marked nondeterministic | `cli::write_partial_on_timeout_writes_partial_session` + `cli::watchdog_timeout_partial_marked_nondeterministic` |
| Session restore does not require original ROM or `.sym` sidecars | `cli::exec_restores_without_rom_or_sym_sidecar` |
| Session embeds the original ROM bytes; `rom_sha256` matches; `Snapshot` lineage cross-check fires on tampering | `cli::session_contains_original_rom_bytes` + `cli::rom_sha256_matches_embedded_rom_blob` + `cli::exec_rejects_rom_hash_mismatch` + `cli::exec_rejects_snapshot_lineage_mismatch` |
| `init` post-load PC is `0x0100` | `cli::init_post_load_pc_is_0x0100` |
| CLI argument errors emit JSON, not clap prose | `cli::arg_error_emits_json_error_envelope` + `cli::arg_error_emits_no_human_prose` |
| JS-produced JSON values are recursively canonicalized | `output::js_value_canonicalization_recursive` + `output::js_object_result_keys_are_sorted` + `output::log_data_keys_are_sorted` |
| `NaN`/`Infinity`/cyclic JS results become `null` plus warning | `output::nan_and_infinity_warn` + `output::cyclic_result_warns` |
| `exec --in same.gbsess --out same.gbsess` is rejected | `cli::exec_in_out_same_path_rejected` |
| `Session::write_new` rejects an existing path; `Session::replace` is gated by `--replace-existing-out` | `cli::write_new_rejects_existing_out` + `cli::replace_existing_out_replaces` |
| Atomic rename behavior is tested on every supported OS triple | `session::atomic_write_replace_semantics` |
| rquickjs feature set is pinned and audited | review packet `cargo tree -e features -p gbf-debug` |
| `gbf-debug` introduces zero `unsafe` lines (excluding rquickjs upstream) | `grep -R "unsafe" gbf-debug/src` returns nothing; CI gate |
| `#![forbid(unsafe_code)]` is in `lib.rs` and `bin/gbf-debug.rs` | source check (CI step) |
| Agent skill validates per agentskills.io spec | `skills-ref validate .agents/skills/gbf-debug-usage` (CI step) |
| Agent skill body fits the progressive-disclosure budget | `wc -l .agents/skills/gbf-debug-usage/SKILL.md ≤ 500` (CI step) |
| Every checked-in recipe under `assets/recipes/` runs end-to-end | `cargo test -p gbf-debug -- skill::recipes_run_against_tiny_rom` |
| CLAUDE.md references the new skill | `grep -q "gbf-debug-usage" CLAUDE.md` (CI step) |

## 13. References

### 13.1 Internal

- `history/planv0.md` — line 158 (crate description), lines 196 + 208 (module list `gbf-debug::{session, script, cli}`), lines 313–321 (`gbf-emu` and `gbf-debug` ownership), lines 2716–2734 (agent debugger design rationale, JS host surface, `.sym` format), line 2904 (M0 deliverable), line 2997 (rquickjs reference).
- `CONSTITUTION.md` — §I.1, §I.2, §III, §IV.3, §V.1, §V.3, §VI.1.
- `.agents/skills/qat-bead-closure/SKILL.md` — closure-skill checklist (claim-to-gate matrix, no future variant rule).
- `bd-1o08` (F-A8 feature bead) and child tasks `bd-1ckj`, `bd-2ulg`, `bd-3psj`, `bd-3shw`, `bd-7fft`, `bd-2i1i`, `bd-24ju`, `bd-1aaz` (T-A8.8a agent skill), `bd-2j4m` (T-A8.8b runtime-ASM conformance smoke suite, follow-up PR).
- `bd-3mxe` (F-A7 feature bead) — F-A8 depends on the public surface F-A7 ships.
- `bd-ssm` (F-A1 feature bead) — F-A8 consumes `parse_sym_entries`, `SymEntry`, the `.gb` ROM byte stream produced by the ROM builder.
- Existing source: `gbf-asm/src/symbols.rs` (`parse_sym_entries`, `SymEntry`, `SymError`), `gbf-emu/src/{adapter,determinism,harness,primitives,trace_ring,trap}.rs` (the six F-A7 modules; F-A7 closed 2026-05-03).
- Sister RFCs: `history/rfcs/F-A3-gbf-abi.md` (the layout-and-discriminants pattern this RFC mirrors at the structural level), `history/rfcs/F-A4-banklease-banking.md` (the single-PR + claim-to-gate matrix pattern this RFC mirrors), `history/rfcs/F-A6-gbf-store-migrate.md` (the deferral-of-migration argument this RFC reuses for session-schema migration).

### 13.2 External

- rquickjs (Rust bindings to QuickJS-NG): <https://github.com/delskayn/rquickjs>
- QuickJS-NG: <https://github.com/quickjs-ng/quickjs>
- gameroy emulator: <https://github.com/Rodrigodd/gameroy>
- zstd reference manual (`level=3`): <https://facebook.github.io/zstd/zstd_manual.html>
- Pan Docs cartridge header (post-bootrom PC = `$0100`): <https://gbdev.io/pandocs/Power_Up_Sequence.html>
- Pan Docs joypad register: <https://gbdev.io/pandocs/Joypad_Input.html>
- RGBDS `.sym` format reference: <https://rgbds.gbdev.io/docs/v0.7.0/rgblink.1#SYMBOL_FILES>
- Agent Skills specification: <https://agentskills.io/specification>
- `skills-ref` reference validator: <https://github.com/agentskills/agentskills/tree/main/skills-ref>

## 14. Review packet requirements

The F-A8 review packet is the engineer's pre-digestion of this RFC for the reviewer. Its job is to let the reviewer verify every load-bearing claim without having to re-derive the design from the diff. **The contents below are mandatory; the file layout, scripts, formats, diagram tools, and exact artifact set are the engineer's call once the implementation is in hand and they can see what naturally falls out of the code.**

### 14.1 Required content areas

The packet must cover each of the following. Each item describes *what* must be conveyed, not *how* the engineer chooses to deliver it.

1. **Orientation.** Short landing page pointing at this RFC, the closed beads, the scope, and a recommended reading path (session schema → JS host → `gb` binding → CLI).
2. **Scope ledger.** What is in F-A8 and what is explicitly deferred. Each deferred item names: the deferred subject; why it is not in F-A8; the owning feature/bead (or "future bead, undecided"); and the F-A8 guard (test or type) that prevents accidental dependence.
3. **Reading-order / multi-pass guide.** A layered walkthrough so a reviewer doesn't have to hold the whole crate in one pass. At minimum the passes should cover: session schema invariants → JS host determinism guards → `gb` object surface (one method per row with its acceptance test) → CLI envelope shapes → end-to-end `init → exec → inspect`.
4. **Diff map.** One row per touched file with a risk rating, a one-line reason a reviewer should care, and the load-bearing tests that gate the file.
5. **Architecture diagrams.** At minimum: a crate-relationship map (this crate vs. its consumers and dependencies); the on-disk `.gbsess` byte layout; the agent loop sequence diagram (write_script → exec → read envelope → repeat); the `gb` object class hierarchy; the JS host determinism-guard table. Tooling and rendering choices are the engineer's call.
6. **Correctness dossier.** For the session schema: every typed enum's variants and the round-trip invariant. For the JS host: the four determinism guards (`Date.now`, `Math.random`, `console`, wall-clock timeout) and the test that gates each. For the `gb` binding: the public method list with the rejection rule for invalid inputs. For the CLI: the exit-code table and the atomic-write protocol.
7. **Claim-to-gate mapping.** Every load-bearing RFC claim (start from §12) mapped to at least one gating test, type invariant, or generated artifact.
8. **Test coverage report.** What tests exist, what they assert, and how to run them in each configuration (default, with `--features ...` if any are added in future). The end-to-end test against a checked-in tiny ROM is called out explicitly.
9. **Reproducibility report.** Pinned toolchain, lockfile, host triple; deterministic-build evidence; the explanation that `SessionMetadata::created_at_micros_since_init` is virtual time (not wall-clock and not `SOURCE_DATE_EPOCH`-seeded); the SHA-256 of the canonical fixture session after one `init` and one `exec`.
10. **Wire-format evidence.** Actual byte-by-byte breakdown of the `.gbsess` magic + flags + zstd frame; an example `Session` JSON pretty-printed; the `OutputEnvelope` shapes pretty-printed.
11. **Generated-artifacts guide.** For each generated artifact (such as the fixture ROM, the fixture session, the symbol table dump), what it is, how it was built, and how to regenerate. The exact set is the engineer's call; it must be sufficient to verify §6's invariants.
12. **Dependency report.** Full dependency tree, evidence that no upward dependency on `gbf-runtime` / `gbf-codegen` / `gbf-artifact` / `gbf-train` exists, license summary, and confirmation that the `gameroy-core` dependency enters only through `gbf-emu`. Use F-A7's recorded `gameroy-core` license in the report; do not assert a transitive license for `gameroy-core` unless the F-A7 dependency report says so.
13. **Known-debt ledger.** Every TODO/FIXME/punt with owner and removal condition. Specifically: the `BuildIdentityBlock` decode follow-up (gated on F-A5 emitting it), the `gb.harness.send(op)` accessor (gated on F-D2), the optional `inspect --full` flag, the optional `gb.disable_breakpoint(addr)` convenience method.
14. **Out-of-scope ledger.** Every item explicitly deferred to a downstream feature, named with the owning feature (mirrors §1.2).
15. **API guide.** What the binary exposes (subcommands, flags, envelope shapes); what the library exposes (the `run_*` functions and the typed envelopes); what is *not* a stable surface (`script::*` internals).
16. **Error-shape report.** Every typed error variant from §5, what triggers it, and how the operator/agent is expected to recover.
17. **Reviewer checklist.** A single-page tickbox covering the load-bearing invariants a reviewer must confirm before approving.
18. **Source-to-artifact traceability.** A worked trace from a single agent invocation through `init` → `exec` → the resulting envelope, with the exact `gbf-debug` invocations and the exact bytes that result.
19. **Optional supplemental videos.** Short walkthroughs with transcripts and exact reproduction commands. The engineer decides whether the complexity warrants them; if shipped, they supplement the written packet, never replace it.

### 14.2 Reproducibility (the one hard rule)

The whole packet must be regenerable from a clean checkout by running a single command:

```bash
cargo xtask regen-review-packet --feature F-A8
```

If `xtask` does not yet exist in the workspace, adding the `xtask` entry point is in scope for the F-A8 PR. The implementation details and artifact formats are the engineer's call. The command name is fixed so CI and reviewers have a stable entry point. Staleness fails loudly.

### 14.3 Acceptance bar

The packet is complete only when:

- A fresh checkout regenerates the packet successfully via the single regen command.
- Every load-bearing RFC claim maps to a test, type invariant, or generated artifact.
- The packet pre-digests session schema, JS host determinism, the `gb` surface, and the CLI envelope shapes — a reviewer should not have to rediscover these from the diff.
- The end-to-end `init → exec → inspect` against the tiny ROM is reproducible from a fresh checkout.
- The "no `unsafe`" invariant is verifiable from the packet (`#![forbid(unsafe_code)]` plus grep evidence).
- The "no upward dependency" invariant is verifiable from `cargo tree`.
- The deterministic-output golden test passes on each supported triple.
- Each in-scope content area in §14.1 is covered.

### 14.4 Core principle

> The engineer should not make the reviewer rediscover the session schema, the JS host's determinism guards, the `gb` surface, or the CLI envelope shapes from the diff. The packet pre-digests all of that, while still giving the reviewer enough precise links, commands, and evidence to independently verify every claim.

The form of that pre-digestion — directory layout, file formats, script names, diagram tools, video format, even the exact set of generated artifacts — is the engineer's call after the implementation lands and they can see what the code naturally produces.

## 15. Appendix: file-by-file change set

| File                                    | Change             | Lines (est.) |
|-----------------------------------------|--------------------|-------------:|
| `Cargo.toml` (workspace)                | Add `"gbf-debug"` to `members`           | +1          |
| `gbf-debug/Cargo.toml`                  | New                | ~30          |
| `gbf-debug/src/lib.rs`                  | New                | ~80          |
| `gbf-debug/src/session.rs`              | New                | ~600         |
| `gbf-debug/src/script.rs`               | New                | ~350         |
| `gbf-debug/src/cli.rs`                  | New                | ~500         |
| `gbf-debug/src/bin/gbf-debug.rs`        | New (clap dispatcher) | ~150        |
| `gbf-debug/tests/session_round_trip.rs` | New                | ~250         |
| `gbf-debug/tests/script_host.rs`        | New                | ~300         |
| `gbf-debug/tests/gb_binding.rs`         | New                | ~500         |
| `gbf-debug/tests/cli_smoke.rs`          | New                | ~250         |
| `gbf-debug/tests/output_envelopes.rs`   | New                | ~200         |
| `gbf-debug/tests/e2e.rs`                | New                | ~200         |
| `gbf-debug/tests/determinism.rs`        | New                | ~100         |
| `gbf-debug/tests/fixtures/`             | New (tiny ROM, fixture sessions, fixture scripts) | ~(binary, regenerable) |
| `gbf-debug/tests/skill_recipes.rs`      | New (runs every `assets/recipes/*.js` under `gbf-debug exec`) | ~150         |
| `.agents/skills/gbf-debug-usage/SKILL.md`           | New (frontmatter + body, ≤ 500 lines) | ≤500         |
| `.agents/skills/gbf-debug-usage/references/ENVELOPE.md`    | New | ~150         |
| `.agents/skills/gbf-debug-usage/references/GB_OBJECT.md`   | New | ~250         |
| `.agents/skills/gbf-debug-usage/references/PREDICATES.md`  | New | ~120         |
| `.agents/skills/gbf-debug-usage/references/DETERMINISM.md` | New | ~120         |
| `.agents/skills/gbf-debug-usage/references/RECIPES.md`     | New (index of recipes) | ~80          |
| `.agents/skills/gbf-debug-usage/assets/recipes/*.js` | New (~5 recipes) | ~250 total   |
| `CLAUDE.md`                             | Add one bullet pointing at `.agents/skills/gbf-debug-usage/SKILL.md` | +2 |
| Review packet (per §14)                 | New (paths, scripts, examples, docs, diagrams chosen by the engineer once the implementation lands) | (engineer's call) |

**Total implementation surface (excluding the engineer-shaped review packet): ~4500 LOC including the agent skill, ~55% of which is tests + skill assets.**

## 16. End

This RFC stays inside the F-A8 boundary. Anything that requires F-A5's runtime nucleus, F-D2's harness control plane, F-A3's `BuildIdentityBlock` decode logic, F-D3's trace transport, or `gbf-store`-backed session storage is explicitly deferred. The proposal lets F-A8 close without those features existing, while leaving every seam (the `metadata.abi_version_observed` field, the eventual `gb.harness` accessor, the trace ring's interoperability with F-D3 transport) shaped for them to plug in cleanly.

Reviewer asks I would value most:

1. **Should closure predicates be invocation-local only?** This RFC now treats closure predicates as non-persisted by default because persisting them as unconditional breakpoints would silently change semantics ("break when A == 0x42" → "break always"). Reviewers should challenge that only if a real workflow requires closure-created breakpoints to survive across `exec`.
2. **Should `gb.write` be `Privileged`-only or always allowed?** The current design always allows it. The agent can write to MBC registers, ROM banks (no-op on cartridge but visible in the emulator's view), and HRAM. This is a debugger and the agent is trusted; the alternative ("require an explicit `gb.privileged_write`") buys safety against typos but slows the loop. Flag if the safety-against-typos argument outweighs.
3. **Is the 32-snapshot soft cap reasonable?** A long script that branches deeply (e.g., binary-searching a 16-step sequence) needs more. The cap is configurable; the question is what the default should be.
4. **Should `inspect` decode any of the F-A3 `BuildIdentityBlock` / `LivenessCounters` / `FaultCode` fields by reading WRAM at known offsets?** The current design says no — that wiring is gated on F-A5 actually emitting them. But if F-A5 is on the critical path with F-A8, doing a "soft" decode that returns `null` for missing magic might be cheap. Flag if it's worth the additional surface in M0.
5. **Anything in the claim-to-gate matrix (§12) missing for closure?** Specifically anything around the `init → exec → inspect` end-to-end loop that F-A8 should pre-test rather than declare-by-contract.
6. **Should the session embed ROM bytes explicitly?** This RFC now does so to preserve the self-contained `.gbsess` contract. Reviewers should challenge this only with an explicit F-A7 save-state contract proving restore works without ROM sidecars (which the current F-A7 RFC does not promise).
7. **Is the default `default_run_budget` (`Machine(MCycles(1_000_000))`) high enough?** `run_until*` has an effective `max_m_cycles` even when JS omits it. Reviewers should check that the default supports F-A5 boot debugging without turning accidental infinite loops into hangs. M-cycle counts for the F-A5 boot path will be a useful signal once F-A5 lands.
