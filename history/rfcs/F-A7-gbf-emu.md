# RFC F-A7: `gbf-emu` — gameroy adapter, DeterminismPolicy, trap dispatcher, trace normalization, harness plumbing

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A7)                                   |
| Status         | Draft; **requires adapter spike before implementation approval** (the spike pins the exact `gameroy-core` API surface; see §3.7 and §0.1) |
| Feature bead   | `bd-3mxe`                                                          |
| Open tasks     | T-A7.1 (`bd-1t5d`, scaffold + gameroy dep), T-A7.2 (`bd-1aql`, core API), T-A7.3 (`bd-10y1`, DeterminismPolicy), T-A7.4 (`bd-19as`, trap dispatcher), T-A7.5 (`bd-14yy`, trace normalization), T-A7.6 (`bd-16z8`, harness-mode plumbing) |
| Closed tasks   | none — every module under `gbf-emu/src/` is still `//! Module stub.` |
| Plan reference | `history/planv0.md` line 157 (workspace `gbf-emu` slot), line 196 (gameroy-backed primitives + breakpoint/watchpoint trapping; `gbf-debug` lives one crate over), line 207 (`gbf-emu::{adapter, primitives, trap, trace_ring, harness, determinism}`), line 313 (gameroy as the sole Rust-native core; library-API shape; registry-driven trap dispatcher; `DeterminismPolicy`), lines 2715–2720 (single-backend rationale; two-backend split is dropped), line 2729 (determinism is a `gbf-emu` policy), line 2901 (M0 scope) |
| Glossary       | `history/glossary.md` (uses existing terms; introduces no new RFC vocabulary) |
| Constitution   | `CONSTITUTION.md` §I (correctness by construction), §III (shifting left), §IV.3 (reproducibility), §V.3 (silent on success / loud on failure), §VI.1 (single source of truth) |

## Project orientation: where this feature sits

### 0.0.1 The big picture

`gbllm5` is an end-to-end Rust toolchain that compiles a quantized transformer down to a deterministic, agent-debuggable Game Boy ROM and runs it on a stock DMG-class device with an MBC5 cartridge. `planv0.md` (line 1 onward) decomposes the system into **five cooperating products plus three shared contracts**:

- **Five products**: a Burn-fronted Rust training stack; a frozen artifact / `CompileRequest` boundary; a three-stratum oracle stack (`DenotationalOracle`, `ArtifactOracle`, `ScheduleOracle`); a staged compiler; and a Bank0 cooperative runtime.
- **Three shared contracts**: `gbf-hw` (the **target contract**), `gbf-artifact` (the **durable model contract**), and `gbf-abi` (the **live execution contract**).

`gbf-emu` is not a contract — it is a *primitive consumer*. It is the single Rust-native execution substrate that every other crate uses when it needs to actually run a Game Boy ROM. `gbf-test`, `gbf-bench`, `gbf-debug`, and `gbf-codegen`'s Trace builds all run their ROMs through `gbf-emu`. There is exactly one such substrate in the workspace, on purpose: the previously-sketched two-backend SameBoy+BGB split has been dropped (planv0.md lines 2715–2720) because gameroy already passes most mooneye-test-suite accuracy gates and maintaining two adapters and two calibration bundles in a Rust-only project did not pay for itself. ([planv0 line 313], [planv0 line 2717])

The project is delivered in milestones M0 → M6:

- **M0** (this RFC's milestone, planv0 line 2901): bring up the foundation stack — `gbf-asm` (typed eDSL, F-A1, shipped), `gbf-hw` (target contract, F-A2), `gbf-abi` (live execution contract, F-A3), `BankLease`/`BankGuard` ABI (F-A4), Bank0 cooperative runtime (F-A5), `gbf-store`/`gbf-migrate` infrastructure (F-A6), `gbf-emu` deterministic emulator adapter (this), `gbf-debug` agent CLI (F-A8). The deliverable is a ROM that boots, draws text, accepts keyboard input, and is debuggable from a script.
- **M1**: oracle stack + first quantized dense kernel + first `CompileRequest`.
- **M2** through **M6**: shared micro-kernels, expert dispatch, sequence state, full interactive text generation, and finally the calibration/autotune loop (where `gbf-bench` produces the concrete calibration bundles whose schema F-A2 ships and whose measurements *all flow through `gbf-emu`*).

`gbf-emu` is a *mid-graph* node in the dependency graph: it depends on `gbf-foundation`, `gbf-hw`, `gbf-abi`, and the external `gameroy` crate, and it is depended on by `gbf-test`, `gbf-bench`, `gbf-debug`, and `gbf-codegen` (Trace builds). Like F-A2 and F-A3, F-A7 ships only primitives — no end-to-end harness loop, no scripting host, no calibration bundle production. Those are owned by the consumer crates.

### 0.0.2 What this feature is for

F-A7 fills the `gbf-emu` crate with the executable substrate every other M0 consumer needs in order to run a Game Boy ROM in a controlled, observable, deterministic way. Concretely, F-A7 owns six things:

1. A `gameroy-core`-backed `Emulator` value with a small, deliberate library API: `load_rom`, `step`, `run_for(budget)`, `run_until_pc(pc, budget)`, register read/write, **bus** vs **debugger** memory access (`bus_read`/`bus_write` vs `peek`/`poke`), snapshot/restore, framebuffer, joypad injection.
2. A `DeterminismPolicy` value object — fixed cartridge RTC (a no-op for MBC5; live for future MBC3/HuC-3), fixed save-state metadata timestamp, typed `PowerOnRamPolicy`, host audio output disabled by default — that **every consumer uses by default**, with a builder for explicit opt-out (e.g., UI smoke tests that need real-time-like behavior).
3. A registry-driven `TrapDispatcher` for PC traps and memory-access traps (read / write / rw kinds), with both closure-shaped predicates (in-script use) and stringified-source predicates (cross-invocation persistence consumed by F-A8's session file).
4. A canonical `NormalizedTraceEvent` format covering memory writes, ROM bank switches, SRAM bank switches, IO writes, and other trace-relevant events, plus the small ring-reader that turns whichever access-observation mechanism the §3.7 adapter spike proves (e.g., the `io_trace` buffer, debugger internals, interpreter instrumentation, or an F-A7-local read/write wrapper) into that event stream.
5. A `HarnessChannel` that lets host-side test/bench/debug code read and write the SRAM-resident `HarnessCommandBlock` / `HarnessResultBlock` (whose `#[repr(C)]` layout is owned by F-A3) and drive the doorbell. F-A7 is plumbing only; the full `HarnessOp` dispatch (`StepSlice` / `RunUntilCheckpoint` / `DumpArena` / `InjectFault` / `PowerCut`) is F-D2's deliverable.
6. A small, exhaustive `EmuError` enum so that "the emulator refused" is always typed.

F-A7 unblocks four immediate features and one whole epic:

- **F-A8 (`gbf-debug`)**: builds the rquickjs-scripted agent CLI on top of `Emulator`, `TrapDispatcher`, `NormalizedTraceEvent`, and snapshot/restore. The session file format and the `gb` JS object layer are F-A8's; F-A7 ships the Rust primitives F-A8 binds.
- **F-E2 (`PlatformCalibrationBundle` production)**: every measurement `gbf-bench` produces runs through an `Emulator` configured with a `DeterminismPolicy`, with PC and memory traps marking the start/end of measurement windows.
- **F-D2 (Harness Control Plane)**: drives `HarnessOp` from the host side via the F-A7 `HarnessChannel`. The full `HarnessOp` set is F-D2's; F-A7's plumbing is the substrate.
- **F-D3 (Trace Pipeline)**: ingests `NormalizedTraceEvent`s into the SRAM ring buffer for downstream consumers; F-A7 ships only the event format and the per-cycle hook plumbing.
- **Epic D `Harness, Persistence, Trace, FailurePolicy`**: every host-side bite F-D2/F-D3/F-D5 takes is one tool call away from `gbf-emu`.

### 0.0.3 Why a single-backend, single-substrate posture

The dominant failure mode this crate exists to prevent is *emulator-side bugs that manifest in only one consumer*. A subtle bug in how the adapter handles `HALT`, or in how RNG is seeded, or in how interrupts fire after `EI; HALT`, can silently differ between "the test that ran" and "the bench that measured" if each consumer wraps the emulator differently. Solving that does **not** require two emulator backends — that was the load-bearing assumption planv0.md walked back at lines 2715–2720. It requires that *all* consumers share exactly one execution substrate with exactly one determinism policy. F-A7 is that substrate.

The differential ladder still has its bites: `ScheduleOracle` ↔ harness, `ArtifactOracle` ↔ `ScheduleOracle`, conformance vs `DenotationalOracle`. Two emulator cores were never the load-bearing safety net. ([planv0 line 2720])

### 0.0.4 What this feature deliberately does *not* do

F-A7 is a single, headless adapter plus its policy and trap primitives. It is not the agent CLI, not the scripting host, not the session file format, not the harness control-plane semantics, not the trace transport ring, and not the calibration-bundle producer. Each of those has a named owner:

- **No JS / scripting host.** That lives in `gbf-debug` (F-A8). `gbf-emu` is consumed by `gbf-debug`, never the other way around. (planv0 line 313: "It does not grow an in-tree emulator core, and it does not host a JS runtime — that lives one crate over.")
- **No `session.gbsess` file format.** That is F-A8's on-disk schema. F-A7 ships only the `Snapshot` value type (a wrapped gameroy save state blob plus an `EmuVersionTag`); F-A8 wraps it together with `SymbolTable`, breakpoints, trace ring, and lineage to forge the on-disk session.
- **No `HarnessOp` semantics.** F-A7's `HarnessChannel` reads the `HarnessCommandBlock` magic + sequence + opcode, dispatches the *transport* (host-side memory I/O, doorbell), and surfaces the result block. The actual `StepSlice` / `RunUntilCheckpoint` / `DumpArena` / `InjectFault` / `PowerCut` semantics are F-D2's.
- **No calibration bundle production.** Owned by `gbf-bench` (Epic E). F-A7's `Emulator` is the substrate; concrete `MeasuredKernelProfile`s, cycle-model drift reports, and Pareto frontiers come out of `gbf-bench`. F-A7 has zero dev-dependencies on `gbf-bench`.
- **No mooneye-test-suite conformance.** That is upstream gameroy's responsibility. F-A7 is allowed to assume gameroy's existing accuracy claims; the differential ladder catches divergence that matters at the workspace level. ([planv0 line 2720])
- **No CGB.** DMG/MGB only for M0. `ConsoleModel::Cgb` exists in `gbf-hw` for future use; F-A7's bring-up profile and tests target DMG.

The boundary is enforced by the dependency graph, not by code review.

### 0.0.5 What changed since this RFC was first sketched

Three substantive changes between the sketch in the F-A7 bead (`bd-3mxe`, comment dated 2026-04-30) and this RFC:

1. **Module split snaps to planv0.md line 207.** The bead's T-A7.1 comment referenced a four-or-five-module split (`adapter/policy/traps/trace/harness`); planv0.md line 207 commits to a six-module split (`adapter, primitives, trap, trace_ring, harness, determinism`). This RFC adopts the planv0.md naming because it is the authoritative one and because it cleanly separates the gameroy-coupled `adapter` from the `primitives` interface every consumer talks in. The current scaffold (`gbf-emu/src/{adapters, breakpoints, harness, trace}.rs`, all `//! Module stub.`) is renamed in this PR; see §3.2 and §18.
2. **`HarnessChannel` is plumbing, not policy.** The bead originally described "host-side StepSlice/RunUntilCheckpoint/DumpArena/PowerCut from host." That is the F-D2 control-plane *content*, not F-A7's transport. F-A7 ships exactly one type that knows how to read `HarnessCommandBlock` from a memory address, parse it against `gbf-abi`'s `#[repr(C)]` layout, write a `HarnessResultBlock` back, and ring the doorbell. The content (which `HarnessOp` does what, who validates, how `FaultCode::HarnessProtocolError` is raised) is F-D2.
3. **F-A7 explicitly carries `gbf-abi` as a hard dependency.** F-A3 ships the `#[repr(C)]` layouts for `BuildIdentityBlock`, `HarnessCommandBlock` / `HarnessResultBlock`, and `TraceEvent`. F-A7's harness/trace modules cannot be implemented without those layouts, so the dependency edge is explicit. (The current `gbf-emu/Cargo.toml` already lists `gbf-abi`, `gbf-foundation`, and `gbf-hw`; this RFC adds the `gameroy` crate.)

The migration plan is detailed in §2.2.1 "Closing the F-A1/F-A2 follow-up: live boot validation." Section §3 (module-by-module) walks every module the F-A7 PR populates.

## 0. TL;DR

### 0.1 RFC self-check before implementation

This RFC is ready to implement only if the following are true:

- The dependency is on the headless **`gameroy-core`** package, not the root `gameroy` GUI/application package. The Cargo entry uses a `package = "gameroy-core"` rename so local imports stay `gameroy::...`. `default-features = false`; only the explicitly named features (`io_trace` and whatever the adapter spike proves necessary) are enabled. The pin is by **git revision** (no SemVer-only pin, no `*` version) and is recorded in `Cargo.lock`.
- The exact public API paths into `gameroy-core` (e.g., `gameroy::gameboy::GameBoy`, the cartridge parser, the interpreter step path, the save-state path, the access-observation mechanism) are pinned by a **compile-only adapter spike** (§3.7) before the rest of the RFC's per-module designs are accepted as implementable. The RFC must not name a `gameroy::Console` / `gameroy::Game` type unless the spike proves those names.
- The crate's module layout matches planv0.md line 207: `adapter`, `primitives`, `trap`, `trace_ring`, `harness`, `determinism`. The current four stub files (`adapters.rs`, `breakpoints.rs`, `trace.rs`, `harness.rs`) are renamed/replaced; new files (`primitives.rs`, `determinism.rs`) are added.
- `gbf-emu` is *not* `no_std`. It links `gameroy-core` (which uses `std`) and runs on the host side. `#![forbid(unsafe_code)]` is set at the crate root; any unavoidable `unsafe` (e.g., a `#[repr(C)]` parse helper that needs alignment-aware reading) lives behind an audited feature flag in `gbf-abi`, not here.
- **Cycle units are typed.** F-A7 introduces `ClockCycles(u64)` and `MCycles(u64)` newtypes plus a `CycleBudget` enum. `run_for(budget)` and `run_until_pc(pc, budget)` take `CycleBudget`; cycle-bearing fields in events / outcomes / lineage carry `ClockCycles`. The conversion `From<MCycles> for ClockCycles` is `n * 4`. The adapter's source-of-truth clock is whatever gameroy-core exposes (clock-count-oriented per upstream README); M-cycle views are derived.
- `DeterminismPolicy::default()` returns the locked-down policy: fixed *cartridge* RTC (a no-op for MBC5; live for MBC3/HuC-3 in the future), a `PowerOnRamPolicy::GameroyDefault` (or seeded variant if the adapter spike confirms gameroy-core exposes that knob), host audio output disabled. The builder-shaped opt-outs (`with_real_time_cartridge_rtc()`, `with_audio_output_enabled()`, `with_power_on_ram(PowerOnRamPolicy)`) are explicit verbs, not flag flips. **Save-state metadata** (the timestamp gameroy-core embeds in its save-state header) is also fixed deterministically — without that, byte-equal snapshots are impossible.
- The `Emulator` API exposes a small fixed verb set: `load_rom(bytes, EmulatorConfig)`, `step`, `run_for(budget)`, `run_until_pc(pc, budget)`, register access, **bus** vs **debugger** memory access (`bus_read` / `bus_write` / `peek` / `poke` / `peek_range`), snapshot/restore, framebuffer, joypad, harness polling. No `run_forever`, no thread spawn, no callback registration outside the trap dispatcher.
- `TrapDispatcher` carries `Predicate::Always`, `Predicate::Closure(Box<dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static>)` (in-script use; **single-threaded**; not `Send + Sync`), and `Predicate::Source(String)` (cross-invocation persistence; opaque to F-A7; F-A8's rquickjs host compiles it into `Closure`). Predicates see a `TrapContext` (regs snapshot, PC, optional `MemoryAccess`, clock cycle, an `EmuReadOnlyView` for side-effect-free peeking), not `&Emulator`.
- A separate, serializable `TrapSpec` / `PredicateSpec` is the persistence shape consumed by F-A8 sessions; `Predicate` itself is *not* serialized. `TrapDispatcher::export_persistable_specs()` returns `Vec<TrapSpec>` and refuses to serialize closure-only entries.
- PC traps fire **before** executing the instruction at the registered address. `run_until_pc(pc, budget)` returns immediately if the emulator is already positioned at `pc`. Memory traps fire on the matching access; trace events and cycle counters update after the instruction.
- Trap dispatch into gameroy-core uses whichever access-observation mechanism the adapter spike proves (e.g., the `io_trace` buffer, debugger-breakpoint internals, interpreter instrumentation, or an F-A7-local wrapper). The RFC does not claim a public per-memory-access callback that hasn't been verified.
- `NormalizedTraceEvent` is exhaustive over the events F-D3 needs at M0: memory write (with `bank: BankSnapshot` and `origin: TraceOrigin`), ROM bank switch, SRAM bank switch, IO write, an F-A7-local `TrapHit { trap_id, kind, cycle }`, and a `Typed(TraceEvent)` passthrough for the F-A3 typed channel. F-A7 does *not* invent an F-A3 `TraceEvent::TrapHit` variant.
- `HarnessChannel` wraps the F-A3 `HarnessCommandBlock` / `HarnessResultBlock` directly (it does not re-mirror the fields locally). Polling and writing happen through `Emulator::poll_harness()` / `Emulator::write_harness_result()` so the borrow shape is ergonomic. `HarnessSlot` carries the SRAM bank explicitly. Magic-byte mismatch surfaces as `EmuError::HarnessMagicMismatch`; sequence-number drift surfaces as `EmuError::HarnessSequenceMismatch`. The doorbell byte is an edge/ready signal, not the source of the `u32` sequence.
- Host-originated harness reads/writes go through adapter-internal peek/poke and **do not** emit guest `MemoryWrite` trace events or trigger guest memory traps. `TraceOrigin::HostPoke` is recorded only when explicit debug auditing is enabled.
- `EmuError` is exhaustive and **internally consistent across §0, §1, §11**: `RomLoad`, `Step`, `TrapPredicate`, `SnapshotSave`, `SnapshotLoad`, `SnapshotRomMismatch`, `SnapshotPolicyMismatch`, `SnapshotEmuVersionMismatch`, `HarnessMagicMismatch`, `HarnessSequenceMismatch`, `TraceCapacityExceeded`, `MemoryAccess`, `Determinism`. Budget exhaustion is an **outcome** (`RunOutcome::BudgetElapsed`), not an error. Pre-1.0, adding a new variant is intentionally a source break for downstream exhaustive matches; F-A7 prefers compile errors over silent fallthrough and does not claim variant additions are backward-compatible.
- `cargo deny` is *not* claimed to lint emulator behavior; it inspects the dependency tree only. A custom integration test (`tests/determinism_single_path.rs`) proves the determinism contract.
- The bring-up `examples/load_tiny_rom.rs` consumes a **checked-in golden fixture** at `gbf-emu/tests/fixtures/tiny_rom.gb` (with provenance metadata and a regeneration command). `cargo test -p gbf-emu` does not shell out to `cargo run -p gbf-asm`. The boot test runs from an explicit `BootMode` (post-boot DMG state, or a pinned boot ROM) and asserts PC reaches `$0150` within `MAX_TINY_ROM_BOOT_BUDGET` — closing F-A1's deferred `tiny_rom_boots` at long last.

### 0.2 Summary

`gbf-emu` is the single Rust-native execution substrate for the workspace. Every consumer that needs to run a Game Boy ROM — `gbf-test` (integration tests), `gbf-bench` (PlatformCalibrationBundle production), `gbf-debug` (agent CLI), `gbf-codegen` (Trace builds), and the M1 oracle stack (ScheduleOracle's emulator-backed reference) — depends on it. Today the workspace pins `gbf-emu` as a crate (`gbf-emu/Cargo.toml`, `gbf-emu/src/lib.rs`) with four `//! Module stub.` files (`adapters`, `breakpoints`, `harness`, `trace`) and three local dependencies (`gbf-abi`, `gbf-foundation`, `gbf-hw`) plus `serde`/`serde_json`. F-A1 has shipped `tiny_rom.gb` + `tiny_rom.sym` and *deferred its live-boot test* explicitly to "the follow-up `gbf-emu`/`gbf-debug` feature" (F-A1 §1.2, §11.4); F-A2 ships the target / cartridge / timing / interrupts / joypad / calibration *schema* `gbf-emu` consumes; F-A3 ships the `#[repr(C)]` `BuildIdentityBlock` / `HarnessCommandBlock` / `HarnessResultBlock` / `TraceEvent` layouts the harness and trace modules parse. F-A7 closes all three of those seams in one PR.

The RFC proposes a six-module crate (renamed from the existing four-stub scaffold) plus an example, a determinism integration test, a snapshot round-trip test, and a trap dispatcher conformance suite. The pipeline is *not* a runtime pipeline — it is a synchronous, single-threaded library API: every `Emulator` method takes `&mut self`, returns a typed result, and runs to completion or a typed outcome / error. There is no thread spawn, no async runtime, no global state. The crate is `std`-using because gameroy-core is `std`-using; the crate-root `#![forbid(unsafe_code)]` is a hard local invariant. The dependency direction is one-way: `gbf-emu` depends on `gbf-foundation`, `gbf-hw`, `gbf-abi`, and the external **`gameroy-core`** package (renamed locally to `gameroy`); every other workspace crate that runs a ROM depends on `gbf-emu` and never the other way around.

**The RFC is gated on a one-day adapter spike that proves the exact `gameroy-core` public API surface F-A7 wraps** (§3.7). Until the spike lands, every gameroy type/path named in this RFC is provisional and subject to rename in the implementation PR.

```
gbf-emu/src/
  lib.rs                ──┐  module re-exports + crate-root lints
  adapter.rs              │  T-A7.1 + T-A7.2: gameroy-core wrapping (exact type pinned by spike);
                          │                   load_rom; step; run_for; run_until_pc; framebuffer; joypad
  primitives.rs           │  T-A7.2: Regs, MemAccess, Snapshot, Framebuffer, JoypadFrame, BankSnapshot
  determinism.rs          │  T-A7.3: DeterminismPolicy + DeterminismPolicyBuilder + opt-out verbs
  trap.rs                 │  T-A7.4: TrapKind, TrapAction, TrapDispatcher, Predicate, BreakpointId
  trace_ring.rs           │  T-A7.5: NormalizedTraceEvent, TraceCursor, gameroy-core access-observation adapter
  harness.rs            ──┘  T-A7.6: HarnessChannel, HarnessSlot, doorbell read/write
```

The new modules add roughly 1,200 LOC of production code plus about 1.6 KLOC of fixture-driven and behavior tests. The crate ships exactly one example (`examples/load_tiny_rom.rs`) — its acceptance is gated on a `cargo test -p gbf-emu` matrix plus the F-A1 follow-up `tiny_rom_boots` test now living here.

The six most load-bearing decisions in this RFC are:

1. **The dependency is on `gameroy-core`, not the root `gameroy` package, and the API surface is pinned by a compile spike.** The root `gameroy` package is the GUI/application crate with UI/audio default features; the headless emulator core lives in the `gameroy-core` package whose library target is named `gameroy`. F-A7's `Cargo.toml` uses the `package = "gameroy-core"` rename so local imports stay `gameroy::...`. Before the implementation PR is approved, a one-day adapter spike (§3.7) compiles a tiny program against the pinned revision and pins the exact construction / step / save-state / access-observation paths. No `gameroy::Console` / `gameroy::Game` symbol is named in this RFC unless the spike proves it.
2. **`DeterminismPolicy::default()` is locked-down.** The defaults are *not* "whatever gameroy-core gives us." They are: cartridge RTC pinned (a no-op for MBC5; live for MBC3/HuC-3 in the future); a `PowerOnRamPolicy` whose default is whatever produces byte-equal repeated runs on the pinned core; host audio output disabled (host audio sink not attached — APU emulation state remains live unless the spike proves disabling it is safe); and a fixed save-state metadata timestamp so snapshot bytes are themselves deterministic. Opt-outs are explicit verbs (`with_real_time_cartridge_rtc()`, `with_audio_output_enabled()`, `with_power_on_ram(...)`) — not booleans flipped on a struct field.
3. **Trap predicates take a `TrapContext`, return `Result<bool, TrapPredicateError>`, are `FnMut`, and are single-threaded.** `Predicate::Closure(Box<dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static>)` carries no `Send + Sync` (the entire emulator is single-threaded by design; `Send + Sync` would make F-A8 harder, especially with a JS runtime handle in scope). `Predicate::Source(String)` is opaque to F-A7; F-A8's rquickjs host compiles it into `Closure`. A separate `TrapSpec` / `PredicateSpec` is the serializable persistence shape so F-A8 sessions can round-trip without F-A7 ever serializing a closure.
4. **`NormalizedTraceEvent` is fixed at M0; new public variants are intentional source breaks pre-1.0.** The M0 set is `MemoryWrite { addr, value, region, bank, origin, cycle }`, `RomBankSwitch { from, to, source, cycle }`, `SramBankSwitch { from, to, cycle }`, `IoWrite { reg, value, cycle }`, an F-A7-local `TrapHit { trap_id, kind, cycle }`, and `Typed(TraceEvent)` (passthrough for F-A3's typed channel). The trace cursor preserves every guest-visible event exactly once; duplicate suppression is permitted only when a fixture proves the duplicate is instrumentation noise. Consumers `match` exhaustively; new variants compile-break them on purpose.
5. **`HarnessChannel` does not own the harness semantics.** It owns transport: wrap an F-A3 `HarnessCommandBlock` directly, expose `Emulator::poll_harness()` and `Emulator::write_harness_result()` on the parent `Emulator` (so the borrow shape is ergonomic), and treat the doorbell as an edge signal — the `u32` sequence number lives in the command block. Whoever drives the dispatch is F-D2's control-plane code, not `gbf-emu`. The reason: `gbf-emu` cannot meaningfully execute `RunUntilCheckpoint` because the checkpoint id space (`SemanticCheckpointId` / `CompactCheckpointId`) is build-specific and lives in `semantic_checkpoint_schema.json` (F-F1) — `gbf-emu` does not have a build to look at, only a ROM. Host-originated harness pokes do *not* emit guest `MemoryWrite` trace events or trigger guest memory traps.
6. **`gbf-emu` ships exactly one example, and it closes F-A1's deferred boot test from a pinned `BootMode`.** F-A1's RFC §1.2 explicitly defers `tiny_rom_boots` (PC reaches `$0150` with no fault) to "the follow-up `gbf-emu`/`gbf-debug` feature." F-A7 implements the test as `examples/load_tiny_rom.rs` plus a sibling integration test, *with an explicit `BootMode` choice* (post-boot DMG state, or a pinned boot ROM by SHA-256), reading the ROM bytes from a checked-in golden fixture (no `cargo run -p gbf-asm` shell-out). The deferred TODO becomes a passing test in this PR.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- An `adapter.rs` that wraps the headless `gameroy-core` package (renamed locally to `gameroy`) into a typed `Emulator { inner: <type pinned by §3.7 spike>, policy: DeterminismPolicy, traps: TrapDispatcher, harness: Option<HarnessChannel>, trace: TraceCursor, ... }` value. Every method takes `&mut self`. The pinned dependency is recorded in `Cargo.lock` by **git revision**; the git rev is the load-bearing identity (SemVer alone can stay constant while source moves).
- `Emulator::load_rom(bytes: &[u8], config: EmulatorConfig) -> Result<Self, EmuError>` — accepts raw ROM bytes (or a `&Path`-loading helper that reads the file). Computes the ROM SHA-256 at load time and stores it for snapshot-lineage and harness-identity checks. `EmulatorConfig` carries `policy: DeterminismPolicy`, `boot_mode: BootMode`, `trace_capacity: usize`, `trace_drop_policy: TraceDropPolicy`, and any harness slot defaults.
- `BootMode` enum with `PostBootDmg` (gameroy-core's post-boot DMG state, `pc = 0x0100`, nonzero `clock_count`) and `BootRom(BootRomImage { bytes: Box<[u8; 0x100]>, sha256: Hash256 })` (run from power-on through a pinned boot ROM; the actual 256 bytes are required because `gameroy::gameboy::GameBoy::new` takes `Option<[u8; 0x100]>`, see §3.7.1 finding 1). The `tiny_rom_boots` test fixes one mode explicitly.
- `Emulator::step(&mut self) -> Result<StepOutcome, EmuError>` and `Emulator::run_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError>` — single-step and bounded run. Both return on the first trap hit (with the trap id), on budget elapsed (a normal `RunOutcome::BudgetElapsed`, **not** an error), or on idle (HALT/STOP).
- `Emulator::run_until_pc(&mut self, pc: u16, budget: CycleBudget) -> Result<RunOutcome, EmuError>` — implicit PC trap that fires before the instruction at `pc` executes and unbinds. Returns `Ok(RunOutcome::TrapHit { ... })` immediately if the emulator is already positioned at `pc`.
- Typed cycle units: `ClockCycles(u64)`, `MCycles(u64)`, `CycleBudget` enum with `Clock(ClockCycles)` / `Machine(MCycles)`. `From<MCycles> for ClockCycles` is `n * 4`. The adapter's source-of-truth clock is gameroy-core's clock count (per upstream README); M-cycle views are derived. `Emulator::clock_count() -> ClockCycles` and `Emulator::m_cycle_count_floor() -> MCycles` expose both.
- Register access (`regs() -> Regs`, `set_regs(&mut self, Regs) -> Result<(), EmuError>`) where `Regs` is an F-A7-local POD with a constructor-validated `Flags(u8)` newtype that masks the low nibble. The conversion to/from gameroy-core's internal type is a one-shot in `adapter.rs`.
- **Bus access (side-effecting, guest-CPU-equivalent)**: `bus_read(&mut self, addr) -> Result<u8, EmuError>` and `bus_write(&mut self, addr, value) -> Result<(), EmuError>`. May advance lazy components (e.g., timer state on IO read) exactly as gameroy-core does for guest CPU accesses. Emit guest `MemoryWrite` trace events with `TraceOrigin::GuestCpu`.
- **Debugger access (side-effect-free)**: `peek(&self, addr) -> Result<u8, EmuError>`, `poke(&mut self, addr, value) -> Result<(), EmuError>`, `peek_range(&self, start, len) -> Result<Vec<u8>, EmuError>`. Must not alter timers, PPU, serial, sound, trace cursor, traps, or cycle count. Pokes are recorded as `TraceOrigin::HostPoke` only when explicit debug auditing is enabled; they do not trigger guest memory traps.
- `Snapshot` value object wrapping a gameroy-core save-state blob plus a `SnapshotLineage { rom_sha256, policy_fingerprint, emu_version: EmuVersionTag, cycle_count: ClockCycles }`. `EmuVersionTag` carries `gameroy_package: &'static str`, `gameroy_semver: SemVer`, `gameroy_git_rev: GitSha`, `gbf_emu_version: SemVer`; `gameroy_git_rev` is injected by `build.rs` from cargo metadata, not from `env!("CARGO_PKG_VERSION")`. `Emulator::snapshot(&self) -> Result<Snapshot, EmuError>` and `Emulator::restore(&mut self, snapshot: &Snapshot) -> Result<(), EmuError>`. `Snapshot` captures only gameroy-core execution state; it does **not** capture installed traps, pending trace cursor contents, harness sequence state, or F-A8 session metadata. F-A7 always passes a deterministic save-state metadata timestamp (`FIXED_SAVE_STATE_UNIX_MS`) into gameroy-core; this is **not** the cartridge RTC, it is metadata in gameroy's save-state header.
- `Framebuffer` value object: 160 × 144 four-color (DMG) `[u8; 23_040]` array, plus a `dmg_palette() -> [Color; 4]` accessor. CGB palette is non-goal in M0.
- `JoypadFrame` value object built over `gbf_hw::joypad::Button`; consumers use the active-high view, with the conversion to gameroy-core's joypad representation localized in `adapter.rs`.
- `DeterminismPolicy` value object with a `Default` impl that locks down cartridge RTC, power-on RAM, save-state metadata, and host audio output. `DeterminismPolicyBuilder` exposes opt-outs as named verbs. The policy's `fingerprint() -> Hash256` method gives a stable identifier consumed by `Snapshot::lineage` and `BuildIdentityBlock` parity checks.
- `TrapDispatcher` with `add_pc(addr, predicate, action) -> BreakpointId`, `add_mem_read(range, predicate, action)`, `add_mem_write(range, predicate, action)`, `add_mem_rw(range, predicate, action)`, `remove(BreakpointId) -> bool`, `remove_entry(BreakpointId) -> Option<RemovedTrap>`, `list()`, `export_persistable_specs() -> Result<Vec<TrapSpec>, TrapPersistenceError>`, plus dispatch hooks wired to whichever access-observation mechanism the adapter spike proves. **PC traps fire before the instruction at the registered address executes; memory traps fire at the matching access; trace events and cycle counters update afterward.**
- `Predicate` enum with `Always`, `Closure(Box<dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static>)`, `Source(String)`. Closures are single-threaded (no `Send + Sync`). `TrapContext<'_>` carries `regs: Regs`, `pc: u16`, `access: Option<MemoryAccess>`, `cycle: ClockCycles`, and an `EmuReadOnlyView<'a>` for side-effect-free peeking. Predicates cannot inspect or mutate trap registry, harness state, trace cursor, or cycle counters.
- `TrapSpec { id, kind, action, predicate: PredicateSpec }` and `PredicateSpec { Always, Source(String) }` are the serializable persistence shapes consumed by F-A8 sessions; `Predicate` itself does not derive `Serialize`/`Deserialize`.
- `NormalizedTraceEvent` enum with `MemoryWrite { addr, value, region, bank: BankSnapshot, origin: TraceOrigin, cycle: ClockCycles }`, `RomBankSwitch { from, to, source, cycle }`, `SramBankSwitch { from, to, cycle }`, `IoWrite { reg, value, cycle }`, `TrapHit { trap_id, kind, cycle }` (F-A7-local; **not** an F-A3 `TraceEvent` variant), and `Typed(TraceEvent)` (the F-A3 typed-event passthrough). `TraceOrigin` enum with `GuestCpu`, `Dma`, `HostPoke`. `TraceCursor` drains the access-observation mechanism into this canonical stream and **does not** dedupe identical consecutive guest events; any duplicate suppression must be backed by a fixture proving instrumentation noise.
- `HarnessChannel` wraps an F-A3 `HarnessCommandBlock` directly (`HarnessCommand { block: gbf_abi::harness::HarnessCommandBlock }`) — no local re-mirror of the field layout. `HarnessSlot { sram_bank: u8, command_addr: u16, result_addr: u16, doorbell_addr: u16 }` carries the SRAM bank explicitly. Polling and writing happen through `Emulator::poll_harness() -> Result<Option<HarnessCommand>, EmuError>` and `Emulator::write_harness_result(HarnessResult) -> Result<(), EmuError>`, which use adapter-internal peek/poke so guest traps and trace events are not triggered. The doorbell is an edge/ready signal; the `u32` sequence number lives in the command block. Magic-byte mismatch / sequence mismatch surface as typed `EmuError` variants.
- A typed, exhaustive `EmuError` enum (single canonical inventory): `RomLoad`, `Step`, `TrapPredicate`, `SnapshotSave`, `SnapshotLoad`, `SnapshotRomMismatch { expected, observed }`, `SnapshotPolicyMismatch { expected, observed }`, `SnapshotEmuVersionMismatch { expected, observed }`, `HarnessMagicMismatch { observed: [u8;4], expected: [u8;4] }`, `HarnessSequenceMismatch { observed: u32, expected: u32 }`, `TraceCapacityExceeded`, `MemoryAccess`, `Determinism`. Variants carry the data needed to reproduce the failing input (CONSTITUTION §V.3). Budget exhaustion is **not** an error; it is `RunOutcome::BudgetElapsed`.
- A `cargo test -p gbf-emu` matrix that proves: determinism (same ROM + same policy → byte-equal trace and byte-equal final state across two runs); snapshot round-trip (save → restore → identical state); trap fires (PC trap; memory R / W / RW traps; PC trap fires *before* the instruction at the address); trace normalization (per-event canonical form, including the "two identical consecutive guest writes are two events" regression); and harness magic-mismatch is typed.
- An `examples/load_tiny_rom.rs` plus a sibling `tests/tiny_rom_boots.rs` that close F-A1's deferred `tiny_rom_boots` test (PC reaches `$0150` within `MAX_TINY_ROM_BOOT_BUDGET` from an explicit `BootMode`). The ROM bytes come from a checked-in golden fixture at `gbf-emu/tests/fixtures/tiny_rom.gb`; `cargo test` does not shell out to `cargo run -p gbf-asm`.
- A `tests/determinism_single_path.rs` integration test that runs the same ROM twice with `DeterminismPolicy::default()` and asserts a byte-equal `(framebuffer, regs, mem_dump)` triple after a fixed `ClockCycles` budget. This is the load-bearing "all consumers see the same execution" test planv0.md line 2729 commits to.
- A **compile-only adapter spike** (`tests/gameroy_api_spike.rs` or `examples/gameroy_api_spike.rs`) that pins the exact public-API paths used (construction, step, save/load, access observation). The spike is folded into `adapter.rs` — or deleted in favor of the adapter — once the implementation lands; until then it is the load-bearing proof that this RFC's API names are real.

### 1.2 Non-goals (deferred)

- **The `gbf-debug` agent CLI, scripting host, and `session.gbsess` file format.** F-A8 (`bd-1o08`). F-A7 ships the Rust primitives F-A8 binds (`Emulator`, `Snapshot`, `TrapDispatcher`, `NormalizedTraceEvent`); F-A8 owns rquickjs binding, the `gb` JS object, and the on-disk schema with zstd compression and one-hop `parent_sha256` lineage.
- **`HarnessOp` semantics (`StepSlice` / `RunUntilCheckpoint` / `DumpArena` / `InjectFault` / `PowerCut`).** F-D2 (Harness Control Plane). F-A7 owns transport (read/write/parse the F-A3 layouts, ring the doorbell, surface typed errors); F-D2 owns the dispatch table.
- **Trace transport ring buffer.** F-D3 (Trace Pipeline) owns the SRAM ring buffer, framing, and harness-side reader. F-A7 ships the `NormalizedTraceEvent` shape and the per-cycle hook plumbing only.
- **PlatformCalibrationBundle / KernelCalibrationBundle / RuntimeCalibrationBundle production.** `gbf-bench` (Epic E, F-E2/F-E3). F-A7 has zero `[dev-dependencies]` on `gbf-bench` and no `cfg(test)` measurement code that produces bundle artifacts.
- **CGB / GBC features.** DMG/MGB only for M0. CGB double-speed mode, VRAM DMA semantics, and the CGB palette format are explicitly out. The bring-up profile sets `console = Dmg`.
- **Mooneye-test-suite conformance gates.** Upstream gameroy. The differential ladder (`ScheduleOracle` ↔ harness, `ArtifactOracle` ↔ `ScheduleOracle`, conformance vs `DenotationalOracle`) catches divergence that matters at the workspace level. F-A7 closure does not run mooneye tests as part of `cargo test -p gbf-emu`.
- **Two-backend differential testing.** Dropped at planv0.md lines 2715–2720. The previously-sketched SameBoy+BGB second backend is not added. F-A7 does *not* leave a "second adapter" seam for future re-introduction; the codebase commits to gameroy.
- **Remote / hardware execution.** Optional real-hardware smoke runs (planv0 line 2711) are nightly-only and consume `gbf-emu` exactly as bulk CI does — they are not a separate substrate. F-A7 does not ship hardware drivers.
- **An in-tree emulator core or a JS runtime.** Both are explicitly forbidden by planv0.md line 313.
- **Workspace-wide grep enforcement.** `gbf-test` (when it lands) checks that no other crate imports gameroy directly. F-A7 ships the test as a `#[ignore]`d smoke test colocated with `gbf-emu/tests/`; promoting it to a full workspace gate is a follow-up bead.

## 2. Background and existing state

### 2.1 What is already in tree

`gbf-emu` itself is module-stub-only at the start of F-A7. Its sibling contracts have shipped or are landing in parallel:

**`gbf-emu` (this crate, fully stubbed):**

- `gbf-emu/Cargo.toml` — pinned, `publish = false`, depends on `gbf-abi`, `gbf-foundation`, `gbf-hw`, `serde`, `serde_json`. Does **not** yet depend on `gameroy`; F-A7 closure adds the dependency.
- `gbf-emu/src/lib.rs` — declares `pub mod adapters; pub mod breakpoints; pub mod harness; pub mod trace;` and contains a single doc-comment. F-A7 closure renames the modules to match planv0.md line 207 (see §3.2), adds `#![forbid(unsafe_code)]` at crate root, and adds two new modules (`primitives`, `determinism`).
- `gbf-emu/src/{adapters, breakpoints, harness, trace}.rs` — every file is exactly `//! Module stub.`. None of them are imported anywhere outside the crate.

**`gbf-foundation` (stable):**

- `gbf-foundation/src/ids.rs` exposes the string-ID and numeric-ID newtype macros. F-A7 reuses `Hash256` (for ROM SHA-256 and policy fingerprint), `SemVer` (for `EmuVersionTag::gameroy_semver`), and `GitSha` (a 20-byte newtype; if `gbf-foundation` does not yet expose it, F-A7 adds it as a precondition — a mechanical addition).
- F-A7 closure adds no new IDs to `gbf-foundation`.

**`gbf-hw` (F-A2 — landing in parallel):** F-A7 reads from F-A2's `cartridge_header`, `joypad`, `interrupts`, `mbc5`, and `memory` modules. Specifically: `gbf_hw::joypad::Button` (8-variant enum) is the input to `JoypadFrame`; `gbf_hw::mbc5::{RAMG, BANK1, BANK2, RAMB}` write-band addresses are the substrate for `NormalizedTraceEvent::RomBankSwitch::source`; `gbf_hw::memory::classify(addr)` classifies the target of `MemoryWrite` and `IoWrite`; `gbf_hw::interrupts::IF_REGISTER` / `IE_REGISTER` are the addresses harness sequence-numbering reads. If F-A2 has not landed by F-A7 closure, the relevant constants are **not** locally redeclared — F-A7 blocks on F-A2 the same way every other consumer does. (See §15 risk row.)

**`gbf-abi` (F-A3 — landing in parallel):** F-A7 reads from `gbf_abi::version::BuildIdentityBlock` (for the lineage-hash sanity check on `Snapshot`), `gbf_abi::harness::{HarnessCommandBlock, HarnessResultBlock}` (for `HarnessChannel`), `gbf_abi::trace::TraceEvent` (the typed-channel passthrough variant of `NormalizedTraceEvent`), and `gbf_abi::fault::FaultCode` (only for surfacing emu-side detection of `FaultCode::HarnessProtocolError` to the consumer; F-A7 does not raise faults itself). If F-A3 has not landed, the same blocking rule as F-A2 applies.

**`gbf-asm` (F-A1 — fully shipped):** F-A1 produces `tiny_rom.gb` + `tiny_rom.lst` + `tiny_rom.sym`. F-A7's `examples/load_tiny_rom.rs` and `tests/tiny_rom_boots.rs` consume those bytes via a **checked-in golden fixture** at `gbf-emu/tests/fixtures/tiny_rom.gb` (with provenance metadata recording the F-A1 example version and the regeneration command). `cargo test -p gbf-emu` **does not** shell out to `cargo run -p gbf-asm`. F-A7 does not directly depend on `gbf-asm` at the crate level — that would create a back edge. The test path is parameterized so it can run against any 32 KiB ROM that respects the F-A1 layout.

The `gbf-foundation` dependency is the home for shared ID newtypes and primitives. `gbf-foundation` is assumed stable for F-A7 closure (no new IDs needed).

### 2.2 What is stubbed

All four `gbf-emu` modules. There is no test file under `gbf-emu/tests/`, no `examples/`, and no integration coverage. F-A7 fills all stubs (renaming three of the four files), adds two new modules (`primitives`, `determinism`), adds a `gbf-emu/tests/` directory with one integration test per module plus a cross-module determinism test, and updates `Cargo.toml` to add the `gameroy` dependency, move `serde_json` to `[dev-dependencies]`, and pin `serde` features.

### 2.2.1 Closing F-A1's deferred boot test

F-A1's RFC §1.2 (and §11.4, §13, §15, §16, §17) explicitly defers the live-boot test for `tiny_rom.gb` to "the follow-up `gbf-emu`/`gbf-debug` feature." The deferred test, named `tiny_rom_boots` in F-A1's claim-to-gate matrix (F-A1 §16), is:

> Running `tiny_rom.gb` in the chosen emulator, asserting PC reaches `$0150` with no fault, with a `DeterminismPolicy::default()` configuration.

F-A7 closure picks this up. The implementation is `examples/load_tiny_rom.rs` plus a sibling integration test `tests/tiny_rom_boots.rs`. The test:

1. Reads `gbf-emu/tests/fixtures/tiny_rom.gb` directly with `include_bytes!` (or `std::fs::read`). The fixture is checked into the repository alongside its provenance metadata (`tiny_rom.gb.provenance.toml`), which records the SHA-256, the F-A1 example version that produced it, and the regeneration command (`cargo run -p gbf-asm --example tiny_rom -- --out gbf-emu/tests/fixtures/tiny_rom.gb`). `cargo test -p gbf-emu` **does not** shell out to `cargo run -p gbf-asm`.
2. Constructs an `Emulator` via `Emulator::builder().boot_mode(BootMode::PostBootDmg).policy(DeterminismPolicy::default()).load_rom(&bytes)`. The `BootMode` is named explicitly so the test does not depend on whatever default gameroy-core picks.
3. Calls `Emulator::run_until_pc(0x0150, MAX_TINY_ROM_BOOT_BUDGET)?`. `MAX_TINY_ROM_BOOT_BUDGET` is a `CycleBudget::Clock(ClockCycles(N))` constant defined in the test file; its value is sized generously to cover the F-A1 100-frame intent in clock-cycle units.
4. Asserts the run returned `RunOutcome::TrapHit { trap_id, .. }` (not `BudgetElapsed`, not `Idle`), and that the surfaced trap is the implicit PC trap at `$0150`.

This is the only F-A1 follow-up F-A7 closes. The "no fault" sub-claim is structurally satisfied by the absence of any `EmuError` return; F-A1's structural tests already prove the ROM has correct headers / no overlapping sections / etc. so a no-fault boot is the consequence we are testing.

Why the explicit `BootMode`: gameroy-core's `reset_after_boot`-equivalent path sets post-boot DMG state (`pc = 0x0100`, nonzero `clock_count`), which is a very different starting point from "boot from power-on through the boot ROM." The F-A1 RFC did not specify which one it meant; F-A7 commits to `BootMode::PostBootDmg` for the bring-up test (no boot ROM is required) and leaves `BootMode::BootRom(BootRomImage { bytes, sha256 })` available for future tests that need power-on-to-`$0150` coverage. The `BootRom` variant carries the actual 256 boot-ROM bytes (gameroy-core's `GameBoy::new` requires them) and the SHA-256 is for lineage validation, not construction.

### 2.3 Downstream pressure on this design

```
gbf-emu ──▶ gbf-test       (integration tests run ROMs via Emulator + DeterminismPolicy)
        ──▶ gbf-bench      (PlatformCalibrationBundle production runs ROMs via Emulator + traps)
        ──▶ gbf-debug      (agent CLI binds Emulator/Snapshot/TrapDispatcher into rquickjs)
        ──▶ gbf-codegen    (Trace builds run the produced ROM through Emulator with TraceCursor)
        ──▶ gbf-oracle     (M1 ScheduleOracle uses Emulator as the operational reference)
        ──▶ gbf-runtime    (test-only: validates runtime nucleus on tiny ROMs)
```

Every consumer assumes:

- The `Emulator` is single-threaded, synchronous, and reentrant from a single owner.
- `DeterminismPolicy::default()` produces *the same byte-for-byte trace and final state* every time, on every machine that compiles the workspace, given the same ROM bytes.
- `TrapDispatcher` callbacks fire on the *first* matching cycle and the run halts there; the consumer chooses whether to call `step` or `run_*` again to continue.
- `Snapshot` is opaque to consumers but byte-stable for a given gameroy version + policy fingerprint pair. `Snapshot::lineage()` exposes that pair.
- `NormalizedTraceEvent` round-trips through serde (host side) so F-D3 can persist the trace ring without re-deriving the wire format.
- Adding a new `NormalizedTraceEvent` variant or a new `TrapKind` is *additive* — consumers `match` exhaustively but the enums are not `#[non_exhaustive]` (we want compile errors when consumers fall behind, not silent fallthrough).

### 2.4 Engineering-rule grounding (`planv0.md` §"Engineering rules")

This RFC threads the rules tightly:

- **Rule 1** ("All generated executable code originates from `AsmIR` / `Instr` / audited runtime builders, never from ad hoc byte pushes"). `gbf-emu` does not generate executable code; it executes existing ROMs. The rule does not bind here, but the dual ("the emulator must not invent its own ROM") is honored by `Emulator::load_rom` taking `&[u8]` and refusing `bus_write` / `poke` outside legal address ranges only via gameroy-core's own checks (no F-A7-side rewrite of the ROM bytes).
- **Rule 5** (deterministic, hashed ROM builds). `DeterminismPolicy::fingerprint()` returns a `Hash256` over the full policy. `Snapshot::lineage` carries `(rom_sha256, policy_fingerprint, emu_version)` where `emu_version` includes `gameroy_git_rev`. Two snapshots that disagree on any of those reject `restore` with `SnapshotRomMismatch` / `SnapshotPolicyMismatch` / `SnapshotEmuVersionMismatch`. ([planv0 line 2729])
- **Rule 6** (`no_std + alloc` where practical). `gbf-emu` is **not** `no_std`-eligible; it links `gameroy` (`std`) and runs on the host. The rule is "where practical" — F-A7 explicitly opts out and documents the reason (gameroy is `std`-only). The `#![forbid(unsafe_code)]` requirement still applies and is enforced.
- **Rule 11** (single source of truth). The "do not run a Game Boy ROM through any other code path" property is the entire point of this crate. The RFC does not add a workspace-wide grep test (gbf-test owns workspace gates), but ships a colocated `#[ignore]` smoke test that flags any other crate importing `gameroy` directly.
- **Rule 12** (`unsafe` is forbidden by default). F-A7 ships zero `unsafe` lines. `#![forbid(unsafe_code)]` is enforced at crate root.
- **Rule 14** (terminology discipline). F-A7 uses "operational" exclusively for schedule/runtime behavior. The trace ring is operational; it is not "semantic" or "denotational" output.
- **Rule 17** (verification-critical algorithms have independent slow refs in `gbf-verify`). `gbf-emu` is not a verification-critical algorithm; it is the substrate the verifications run *on*. The rule does not bind, but the structural posture (single substrate, single policy, deterministic) is what makes the slow refs in `gbf-verify` comparable to anything.

### 2.5 Constitutional grounding (`CONSTITUTION.md`)

- **§I.1 (correctness by construction).** `EmuError` and its sub-errors are exhaustive over the failure modes F-A7 cares about. `Predicate` and `TrapKind` are exhaustive enums. `Snapshot::lineage()` returns a value object (not a flag-bag); mismatched lineage is a compile-checked `match` arm in the consumer.
- **§III (shift left).** Determinism is asserted by `tests/determinism_single_path.rs` running two full execution passes and `assert_eq!`ing their `(framebuffer, regs, mem_dump)` triples. Trap dispatch is `match`-checked against the pinned `TrapKind` discriminants.
- **§IV.3 (reproducible builds).** Every byte of the trace ring and every byte of `Snapshot` is reproducible across a fresh checkout, given the same ROM, same gameroy pin, and same `DeterminismPolicy`. The integration test enforces this.
- **§V.3 (silence on success, loud on failure).** `EmuError` carries enough state to reproduce failure: `HarnessSequenceMismatch` carries observed and expected sequence numbers; `SnapshotRomMismatch` carries observed and expected SHA-256; `AddressRangeError::StartAfterEnd` carries the inverted bounds. Budget exhaustion is *not* an error — it is `RunOutcome::BudgetElapsed`, which carries observed and requested clock cycles.
- **§VI.1 (single source of truth).** `gbf-emu` is the single substrate for executing Game Boy ROMs in the workspace. The dependency graph enforces it.

### 2.6 gameroy as the primary specification

The gameroy repository (https://github.com/Rodrigodd/gameroy) is a **workspace** containing two relevant packages:

- The root **`gameroy`** package — the GUI / application crate, with default features pulling in UI / audio dependencies. The library target inside this package is named `gameroy_lib`. F-A7 does **not** depend on this package.
- The **`gameroy-core`** package (under `core/` in the workspace) — the headless emulator core, with the lower-level emulator modules (`gameboy`, `interpreter`, `parser`, `save_state`, …) and an `io_trace` feature. The library target inside this package is named `gameroy`. F-A7 depends on this package, renamed locally so source imports stay `use gameroy::...`.

The dependency entry is therefore:

```toml
gameroy = { package = "gameroy-core", git = "https://github.com/Rodrigodd/gameroy", rev = "<pinned-sha>", default-features = false, features = ["io_trace"] }
```

(Additional features may be enabled if and only if the §3.7 adapter spike proves they are required.)

`gameroy-core` is:

- Independently maintained (not a fork);
- Reported by upstream as passing the Blargg / Mooneye / Mealybug / DMG-Acid suites it lists in its README; the **exact upstream test-status excerpt is recorded in the F-A7 review packet** rather than paraphrased here;
- Exposes a library API we can drive headlessly from `gbf-test`, `gbf-bench`, and the agent debugger (planv0 line 313);
- Pure Rust, **dual-licensed `MIT OR Apache-2.0`** (verified at F-A7 closure time and recorded in the review packet).

F-A7 pins `gameroy-core` to a specific **git revision** (no SemVer-only pin, no branch reference). The pin is recorded in `Cargo.lock`. Bumping the pin is a follow-up bead and produces a fresh `EmuVersionTag::gameroy_git_rev` value, which in turn invalidates every `Snapshot` produced under the old pin (by `SnapshotEmuVersionMismatch`). This is the right behavior: gameroy-core's save-state format is not a stability commitment we make, and consumers that snapshot under one git revision cannot expect to restore under another. SemVer alone is *not* used for snapshot identity because a published version can stay constant while the source moves.

The places where this RFC deliberately deviates from gameroy-core's defaults:

1. **Cartridge RTC.** Some MBC families (MBC3, HuC-3) expose RTC registers that gameroy-core may default to system-time. F-A7's `DeterminismPolicy::default()` pins the cartridge RTC to `2000-01-01T00:00:00Z` (`FIXED_CARTRIDGE_RTC_UNIX_MS = 946_684_800_000`). **For M0's MBC5 target this is a no-op**, since MBC5 has no RTC surface (Pan Docs §"MBC3" describes the RTC; MBC5 does not). The policy field exists for future MBC3/HuC-3 support; for MBC5-only ROMs the value is unobserved.
2. **Save-state metadata timestamp.** gameroy-core's save-state path accepts an optional timestamp and serializes it into the save-state header. If F-A7 ever passes wall-clock time, byte-equal snapshots are impossible even when emulation itself is deterministic. F-A7's `DeterminismPolicy::default()` therefore pins the save-state metadata timestamp to `FIXED_SAVE_STATE_UNIX_MS = 946_684_800_000` (or `None`, if the adapter spike proves `None` encodes deterministically). This is **separate** from the cartridge RTC and is a `gbf-emu`-only concern.
3. **Power-on RAM / host entropy.** F-A7 carries a `PowerOnRamPolicy` (default: `GameroyDefault`, optionally `FixedFill(u8)` or `Seeded { seed: u64 }`). This knob exists to absorb any host-entropy or randomized-uninitialized-RAM behavior the pinned gameroy-core revision exposes; if the adapter spike proves the core is fully deterministic without any seed, the policy variants beyond `GameroyDefault` may be removed in a follow-up. The earlier draft's "RNG seed `u64`" was underspecified and is replaced by this typed policy.
4. **Host audio output.** F-A7's `DeterminismPolicy::default()` disables **host audio output** (no host audio sink attached). It does **not** unconditionally disable APU emulation — APU state may be part of accurate execution evolution and disabling it could change CPU-visible behavior; the adapter spike resolves which level of disable is correct. UI smoke tests that need real audio playback opt out via `with_audio_output_enabled()`.

### 2.7 Relationship to other M0 features

```
                        ┌─────────────────────────┐
                        │  F-A7: gbf-emu          │   ← this RFC
                        │  (gameroy substrate)    │
                        └────────────┬────────────┘
                                     │
        ┌────────────────────────────┼────────────────────────────┐
        ▼                            ▼                            ▼
┌─────────────────┐         ┌─────────────────┐         ┌─────────────────┐
│  F-A8: gbf-debug│         │  F-D2: harness  │         │  F-D3: trace    │
│  (agent CLI +   │         │   control plane │         │   pipeline      │
│   rquickjs +    │         │  (StepSlice/    │         │  (SRAM ring +   │
│   session file) │         │   RunUntilCkpt) │         │   framing)      │
└─────────────────┘         └─────────────────┘         └─────────────────┘
                                     │
                                     ▼
                            ┌─────────────────┐
                            │  F-E2: bench    │
                            │  (Platform-     │
                            │   CalibBundle   │
                            │   production)   │
                            └─────────────────┘
```

F-A7 is *upstream* of F-A8, F-D2, F-D3, and F-E2 inside Epic A / D / E. Closing F-A7 unblocks all of them.

F-A7 is *downstream* of F-A1 (uses `tiny_rom.gb` for the boot test), F-A2 (uses `Button`, MBC5 register addresses, memory-region classification), and F-A3 (uses `BuildIdentityBlock`, harness layouts, `TraceEvent`). If any of those have not landed by F-A7 closure, F-A7 blocks on them — no local re-declaration is permitted.

### 2.8 Beads under this feature

The six child tasks under `bd-3mxe` are:

| Bead     | Task     | Module(s)                | Priority |
|----------|----------|--------------------------|----------|
| `bd-1t5d`| T-A7.1   | scaffold + `Cargo.toml`  | P0       |
| `bd-1aql`| T-A7.2   | `adapter.rs`, `primitives.rs` | P0  |
| `bd-10y1`| T-A7.3   | `determinism.rs`         | P0       |
| `bd-19as`| T-A7.4   | `trap.rs`                | P0       |
| `bd-14yy`| T-A7.5   | `trace_ring.rs`          | P1       |
| `bd-16z8`| T-A7.6   | `harness.rs`             | P1       |

T-A7.1 (scaffold) blocks T-A7.2 and T-A7.3. T-A7.2 (adapter / primitives) blocks T-A7.4 (trap dispatcher needs `Emulator` to hook into) and is independent of T-A7.3 (which only needs the scaffold's `DeterminismPolicy` type stub). T-A7.4 (trap dispatcher) blocks T-A7.5 (trace normalization rides the same per-cycle hook) and T-A7.6 (harness uses memory-write traps for the doorbell). T-A7.3 parallelizes with T-A7.2.

The order matters within the PR's commit history; it does not change the closure shape, which is one PR closing every task.

The bead-level dependency edge (per `bd-3mxe`'s `Dependencies` block) records that F-A7 *blocks* F-A1 (`bd-ssm`) and F-A3 (`bd-2k2`) — those edges encode the **F-A7-blocks-on-them** ordering: F-A7 cannot land until F-A1 and F-A3 have landed because F-A7's tests consume their outputs. (The bead tool's "blocks" terminology is the reverse of common English usage; the human-readable reading is "F-A7's progress is blocked by these.")

## 3. Architecture

### 3.1 Crate-level shape

`gbf-emu` is an *adapter* crate. The entire public surface decomposes into:

1. **`Emulator`** — the central `&mut self`-bearing value object. Owns the gameroy-core emulator handle (the exact type — most likely `gameroy::gameboy::GameBoy` based on the current public surface, but pinned by the §3.7 spike), the `DeterminismPolicy`, the `TrapDispatcher`, the optional `HarnessChannel`, and the `TraceCursor`.
2. **Value objects** — `Snapshot`, `Regs`, `Framebuffer`, `JoypadFrame`, `BankSnapshot`, `EmuVersionTag`, `DeterminismPolicy`, `RunOutcome`, `StepOutcome`, `BreakpointId`, `NormalizedTraceEvent`, `HarnessCommand`, `HarnessResult`. All are `Copy + Clone + Debug + Eq + PartialEq` plain structs / enums except where ownership precludes it (`Snapshot` carries `Vec<u8>`, `HarnessCommand` may carry up to a 32-byte payload).
3. **Builders** — `EmulatorBuilder` (consumed by `Emulator::load_rom`), `DeterminismPolicyBuilder`. Builders are used where the constructor would otherwise have many `Option` parameters; otherwise `T::new(...)` is preferred.
4. **Exhaustive enums** — `EmuError`, `TrapKind`, `TrapAction`, `Predicate`, `NormalizedTraceEvent`, `RunOutcome`, `StepOutcome`, `MemoryAccessKind`. None are `#[non_exhaustive]`.
5. **Access-observation plumbing** — `TraceCursor` and the trap dispatcher's `dispatch(addr, kind, ctx)` glue. Wired into whichever access-observation mechanism the §3.7 spike proves the pinned `gameroy-core` revision exposes (e.g., the `io_trace` buffer, debugger-internal hooks, interpreter instrumentation, or an F-A7-local read/write wrapper).

There is *no* runtime state outside an `Emulator` instance. There are *no* mutable globals. There are *no* IO entry points except through the explicit verbs on `Emulator`. The crate root uses `#![forbid(unsafe_code)]` as the primary mechanism for keeping `unsafe` out — the compiler enforces this at every build, not a fragile grep. The crate is `std`-using (gameroy requires it).

### 3.2 Module responsibility table

| Module           | Owns                                                                                          | Public surface |
|------------------|-----------------------------------------------------------------------------------------------|---------------|
| `adapter.rs`     | `Emulator`, `EmulatorBuilder`, `EmulatorConfig`, `BootMode`, gameroy-core wrapping, `load_rom`, `step`, `run_for`, `run_until_pc`, bus/peek register/memory access, `clock_count`, `m_cycle_count_floor` | ~30 items |
| `primitives.rs`  | `Regs`, `Flags`, `Framebuffer`, `JoypadFrame`, `Snapshot`, `SnapshotLineage`, `EmuVersionTag`, `GitSha`, `BankSnapshot`, `RunOutcome`, `StepOutcome`, `CpuIdleState`, `MemoryAccess`, `MemoryAccessKind`, `ClockCycles`, `MCycles`, `CycleBudget` | ~28 items |
| `determinism.rs` | `DeterminismPolicy`, `DeterminismPolicyBuilder`, `CartridgeRtcMode`, `SaveStateMetadataMode`, `PowerOnRamPolicy`, `AudioOutputMode`, `FIXED_CARTRIDGE_RTC_UNIX_MS`, `FIXED_SAVE_STATE_UNIX_MS`, `fingerprint` | ~14 items |
| `trap.rs`        | `TrapDispatcher`, `TrapKind`, `TrapAction`, `Predicate`, `BreakpointId`, `TrapPredicateError` | ~14 items |
| `trace_ring.rs`  | `NormalizedTraceEvent`, `TraceCursor`, `TraceCursorError`, gameroy-core access-observation adapter | ~10 items |
| `harness.rs`     | `HarnessChannel`, `HarnessSlot`, `HarnessCommand`, `HarnessResult`, doorbell read/write       | ~10 items |

Total public surface: ~94 items. Smaller than F-A2 (~160) and F-A3 (~80) because F-A7's job is to wrap *one* external crate in a typed adapter, not to enumerate hardware constants.

The current scaffold maps to the new layout as follows:

| Old file (stub)              | New module(s)                  | Action |
|------------------------------|--------------------------------|--------|
| `gbf-emu/src/adapters.rs`    | `gbf-emu/src/adapter.rs` (singular) + `primitives.rs` | Rename `adapters.rs` → `adapter.rs`; create new `primitives.rs`. |
| `gbf-emu/src/breakpoints.rs` | `gbf-emu/src/trap.rs`          | Rename. The bead's "breakpoints" is the consumer-facing word; the planv0.md module name is `trap`. |
| `gbf-emu/src/harness.rs`     | `gbf-emu/src/harness.rs`       | Keep filename; replace stub. |
| `gbf-emu/src/trace.rs`       | `gbf-emu/src/trace_ring.rs`    | Rename to match planv0.md line 207. |
| (none)                       | `gbf-emu/src/determinism.rs`   | Create new file. |

The `lib.rs` `pub mod` declarations are updated to match. No external crate currently imports `gbf_emu::adapters::*` / `gbf_emu::breakpoints::*` / `gbf_emu::trace::*` (the modules are stubs with no public items), so the rename has zero downstream impact.

### 3.3 Type-state of `Emulator`

`Emulator` is the keystone type. Every other module's outputs feed into it. Its shape (every gameroy-core type below is **provisional**, pinned by the §3.7 adapter spike):

```rust
pub struct Emulator {
    // The exact gameroy-core type — most likely `gameroy::gameboy::GameBoy` based on
    // the current public surface (`core/src/gameboy.rs` exposes `GameBoy`), but the
    // §3.7 spike is the load-bearing proof.
    inner: gameroy::gameboy::GameBoy,

    policy: DeterminismPolicy,          // immutable after construction
    rom_sha256: Hash256,                // computed at load_rom; immutable
    boot_mode: BootMode,                // immutable after construction
    traps: TrapDispatcher,              // mutable via verbs
    trace: TraceCursor,                 // mutable; capacity from EmulatorConfig
    harness: Option<HarnessChannel>,    // None until consumer attaches
    // No independent cycle counter. The adapter reads gameroy-core's clock_count
    // and converts units. Maintaining a parallel counter desynchronizes from the
    // core across save/restore, post-boot init, and any debugger read that updates
    // lazy components.
}
```

A plain `Emulator` is constructed by `Emulator::load_rom(bytes, EmulatorConfig) -> Result<Self, EmuError>`. The constructor:

1. Computes `rom_sha256 = Hash256::sha256(bytes)`.
2. Builds the policy fingerprint and applies the determinism policy (cartridge RTC for MBC3-class, save-state metadata timestamp, power-on RAM, host-audio sink).
3. Constructs the gameroy-core `GameBoy` from the cartridge bytes via the **spike-pinned** parser/constructor path (`Cartridge::new(bytes.to_vec())` + `GameBoy::new(boot_rom, cartridge)`); transitions to `config.boot_mode` (post-boot DMG state via `GameBoy::new(None, cartridge)` and/or `reset_after_boot`, or boot-ROM execution by passing `Some(boot_rom_image.bytes)` if `BootMode::BootRom(BootRomImage { bytes, sha256 })`; see §3.7.1 finding 1).
4. Initializes an empty `TrapDispatcher`, a `TraceCursor` with `config.trace_capacity` and `config.trace_drop_policy`, and `harness = None`.
5. Returns the constructed `Emulator`.

All mutating operations take `&mut self` and return `Result<_, EmuError>` or a typed outcome. Read-only operations (`regs`, `framebuffer`, `clock_count`, `m_cycle_count_floor`, `rom_sha256`, `policy`, `version_tag`) take `&self` and return values. The `peek` / `peek_range` debugger reads also take `&self` because they are explicitly required to be side-effect-free; the side-effecting `bus_read` takes `&mut self`.

The fields are private; consumers go through accessors. A `#[non_exhaustive]` annotation is *not* applied; the struct is closed.

### 3.4 The "single substrate" boundary

The architectural intent that `gbf-emu` is *the* place for ROM execution is **made visible** by three mechanisms; **enforcement** lands when `gbf-test` does:

1. **Documentation-only smoke test until promoted.** `gbf-emu/tests/single_substrate_smoke.rs` is `#[ignore]`d in F-A7. It walks the workspace and greps every other crate's `Cargo.toml` for `gameroy-core` / `gameroy =` and every other crate's source for `use gameroy::`. While ignored, it does not run by default and **does not enforce anything** — F-A7's claim-to-gate matrix lists it under "documentation/known-debt", not under "load-bearing acceptance gates." The promotion to a non-ignored workspace gate happens in a follow-up bead inside `gbf-test`. (Allowlist entry: `gbf-emu/src/adapter.rs`. False positives go in a YAML allowlist colocated with the test.)
2. **Workspace `Cargo.toml` posture.** `gameroy-core` is added only at `gbf-emu/Cargo.toml`'s `[dependencies]` (no other crate depends on it directly). Adding `gameroy-core` to another crate's `Cargo.toml` requires a comment documenting why, and the smoke test in (1) flags it once promoted.
3. **Doc-link visibility.** Every consumer crate's relevant module carries `//! Authoritative substrate: [`gbf_emu::Emulator`].` doc comments; rustdoc's `--deny broken-intra-doc-links` flag (already enabled on workspace lints) ensures the link resolves.

The dependency graph **makes the boundary visible**; the boundary is enforced only after the workspace gate lands. F-A7 does not claim "the dependency graph enforces the boundary" — that overstates the guarantee.

### 3.5 Why `DeterminismPolicy` lives in `gbf-emu`, not `gbf-hw` or `gbf-bench`

`DeterminismPolicy` *types* are consumed by:

- `gbf-test::Emulator::load_rom` (every test),
- `gbf-bench::PlatformCalibrationBundle::measure` (every measurement),
- `gbf-debug::Session::open` (every CLI invocation),
- `gbf-codegen::trace_build::run` (every Trace build).

If the policy lived in `gbf-hw`, the determinism contract would be a hardware fact — but RTC pinning, RNG seeding, and audio disabling are *emulator* facts, not target facts. They are concerns of how we run a ROM, not what hardware we are running it on. If the policy lived in `gbf-bench`, every consumer would have to depend on `gbf-bench`, which transitively pulls in benchmarking machinery and workload manifests — which is wrong both architecturally and practically (`gbf-test` cannot depend on `gbf-bench`; that would create a cycle through workload manifests).

Putting the policy in `gbf-emu` is the only place where every consumer pays only the substrate cost and the policy stays disjoint from both hardware and bench. ([planv0 line 2729])

### 3.6 What `gbf-emu` deliberately does not own

- **Cycle costs per `Instr`.** `gbf-asm::cycle_model` (F-A1, shipped). `gbf-emu` runs ROMs; per-instruction costs are computed in `gbf-asm` because they are tied to LR35902 instruction encoding.
- **`SemanticCheckpointSchema` / `SemanticCheckpointId` resolution.** `gbf-abi::checkpoint` (F-A3). `gbf-emu`'s `run_until_pc` operates on raw `u16` addresses; mapping a `SemanticCheckpointId` to a PC is the consumer's job (typically F-D2).
- **The `gameroy` crate itself.** F-A7 wraps it; F-A7 does not fork it, vendor it, or mirror it. Bumping the gameroy pin is a follow-up bead.
- **`HarnessOp` semantics.** F-D2 (Harness Control Plane). F-A7's `HarnessChannel` is the wire; F-D2 is the protocol.
- **Trace transport ring buffer.** F-D3 (Trace Pipeline). F-A7's `TraceCursor` is the per-cycle hook drainer; F-D3 owns the SRAM ring and the framing.
- **Calibration bundle production.** `gbf-bench` (Epic E). F-A7 runs ROMs; `gbf-bench` measures and emits.
- **Session file format.** `gbf-debug` (F-A8). F-A7 ships `Snapshot`; F-A8 wraps it.
- **rquickjs scripting host.** F-A8. ([planv0 line 313])

The boundary between `gbf-emu` and these crates is enforced by the dependency graph: `gbf-emu` depends on `gbf-foundation`, `gbf-hw`, `gbf-abi`, and `gameroy-core` (renamed to `gameroy`); `gbf-test`, `gbf-bench`, `gbf-debug`, `gbf-codegen` depend on `gbf-emu`. There is no path back the other way.

### 3.7 Required adapter spike (precondition for implementation approval)

Before the per-module designs in §4–§8 are accepted as implementable, F-A7 lands a **compile-only adapter spike** against the pinned `gameroy-core` revision. The spike's job is to prove the exact public-API paths the rest of this RFC assumes. Until the spike compiles, every gameroy type / path named below is provisional.

The spike must prove:

1. **Package and rename.** `Cargo.toml` uses `gameroy = { package = "gameroy-core", git = "...", rev = "...", default-features = false, features = ["io_trace"] }` and the resulting source imports work as `use gameroy::...`.
2. **Cartridge / `GameBoy` construction.** The exact path to parse cartridge bytes (likely `gameroy::parser::*` and `gameroy::gameboy::GameBoy::new(...)`-shaped, but pinned by the spike) and the post-boot state path (likely `reset_after_boot`-shaped).
3. **Stepping / interpreter.** The exact path to advance one instruction (or N clock cycles) — interpreter step, `tick`, or a higher-level `run_for(cycles)` — and how to read the resulting clock count.
4. **Save state.** The exact path to save and restore via `gameroy::save_state::*` (or whatever the pinned revision exposes), including the optional timestamp parameter.
5. **Access observation.** Whether memory read/write tracing is callback-based, buffer-based (the `io_trace` feature), debugger-internal, or unavailable. The chosen mechanism becomes the substrate the trap dispatcher and trace cursor wire into. If the pinned revision exposes no public per-memory-access callback, the RFC's §6 / §7 designs fall back to a buffer-poll model and the per-cycle-callback wording is rewritten in the implementation PR.
6. **Framebuffer access.** The exact path to read the 160 × 144 DMG framebuffer (and, separately, whether the result is byte-equal across reruns under the pinned policy — this is the substrate the determinism single-path test asserts).
7. **Joypad injection.** The exact path to set the joypad state.
8. **Debugger / breakpoint surface, if any.** Whether gameroy-core's debugger exposes a public breakpoint/watchpoint registry F-A7 can layer on, or whether F-A7 builds its own.

The spike file lives at `gbf-emu/tests/gameroy_api_spike.rs` (compile-only; an `#[ignore]`d test wrapper if needed). It is allowed — and expected — to evolve during the implementation PR; once `adapter.rs` is fully populated, the spike either folds into the adapter's doc-tests or is deleted in the same PR.

The spike is the load-bearing artifact named in this RFC's review packet under "gameroy pin record" and "API guide." If the spike contradicts an assumption in §4–§8, the RFC is amended in the implementation PR rather than the implementation forced to match an unverified shape.

### 3.7.1 API-surface feasibility findings (pre-spike review)

A pre-spike review against `gameroy-core` `master` produced the following load-bearing corrections. These are *not* a substitute for the §3.7 compile spike against the pinned rev, but they are inputs to it: §4–§8 below already reflect them, and the spike's job is to either (a) confirm them against the pinned rev or (b) flag drift in the implementation PR.

1. **`BootMode::BootRom { sha256 }` is insufficient.** `gameroy::gameboy::GameBoy::new(boot_rom: Option<[u8; 0x100]>, cartridge: Cartridge)` requires the actual 256 boot-ROM bytes, not a hash. F-A7 carries the bytes and validates the hash for lineage. Shape: `pub enum BootMode { PostBootDmg, BootRom(BootRomImage) }` with `pub struct BootRomImage { pub bytes: Box<[u8; 0x100]>, pub sha256: Hash256 }`. The post-boot path is `GameBoy::new(None, cartridge)` (or `reset_after_boot`).

2. **`Regs::ime: bool` cannot round-trip gameroy CPU state.** Gameroy's CPU has tri-state IME (`Disabled | Enabled | ToBeEnable`), and the `ToBeEnable` state is observable around `EI` + interrupts. `Regs::ime` becomes a `pub enum ImeSnapshot { Disabled, Enabled, ToBeEnable }`. HALT/STOP execution state is **not** part of `Regs`; it lives in `StepOutcome::Idle{state}` / `RunOutcome::Idle{state}`.

3. **`peek` / `poke` are not side-effect-free across the full bus** (Option B selected). `gameroy-core`'s public `GameBoy::read(&self, addr)` updates lazy IO/timer/PPU/sound state through interior mutability. F-A7 narrows the `peek` / `poke` contract: they are side-effect-free **only** for raw-backed regions (ROM, WRAM, HRAM, cartridge SRAM at an explicit bank, and — if the spike confirms direct access — VRAM/OAM). For unsupported regions (notably IO at `$FF00..=$FF7F`), they return `EmuError::DebugMemoryUnsupported { addr }`. IO inspection goes through `bus_read`, which is side-effecting by definition.

4. **Memory traps under `io_trace` are post-instruction traps, not exact pre-access traps.** `io_trace` is a buffer the adapter drains; there is no public callback that can stop the CPU mid-instruction. F-A7 commits to: PC traps fire **before** instruction execution (matches debugger expectation); memory traps under the `io_trace` backend fire **at the instruction boundary** after the matching access, with the reported emulator state being post-instruction. Exact pre-access memory traps are an explicit non-goal for M0; they require an upstream-instrumented `gb_read` / `gb_write` callback and are deferred to a follow-up bead.

5. **`io_trace` must be drained per-instruction.** The `io_trace` tuple's first element is a `u8` packed with `kind | ((clock_count & !3) >> 1)`, not an absolute `u64`. The adapter clears/drains `io_trace` once per `interpret_op` invocation and reconstructs `ClockCycles` for each event from the step's `start_clock` / `end_clock` plus per-access ordering. The trace ring **never** consumes an `io_trace` buffer accumulated across multiple instructions.

6. **Add `TraceOrigin::HostBus`.** `TraceOrigin` distinguishes guest-CPU accesses (interpreter-driven) from `bus_read` / `bus_write` (host-invoked, side-effecting) and from `poke` (host raw-debug write). Variants: `GuestCpu | HostBus | HostPoke | Dma`.

7. **Several `DeterminismPolicy` knobs do not have public gameroy-core hooks and are removed from M0:**
   - `with_audio_output_enabled()` is **removed** from the builder. `gbf-emu` depends on the headless `gameroy-core` package which has no host audio sink; the policy field becomes locked to `AudioOutputMode::Disabled` for M0. Re-enabling host audio requires depending on the GUI/application package, which this RFC explicitly does not.
   - `PowerOnRamPolicy::Seeded { seed }` is **dropped** for M0 unless the spike proves a seedable upstream hook. `FixedFill(u8)` becomes `FixedFill { wram: u8, hram: u8, cartridge_ram: u8 }` (per-region since gameroy-core defaults to `0xFF` for WRAM/HRAM and `0x00` for cartridge RAM).
   - `CartridgeRtcMode::Fixed` is included in the policy fingerprint for future support but is **applied** only when `gameroy-core` exposes a cartridge RTC hook. For M0 / MBC5 it is unobserved. Loading an RTC-bearing cartridge under F-A7 returns `EmuError::Determinism { reason: "cartridge RTC control unavailable" }` unless RTC control is available in the pinned rev.

8. **Snapshot lineage carries boot-mode fingerprint.** `gameroy-core`'s save state does not include the boot-ROM bytes themselves, so a snapshot taken under one boot-ROM image is under-specified if restored under a different one. `SnapshotLineage` adds `boot: BootModeLineage { PostBootDmg | BootRom { sha256: Hash256 } }`.

9. **Harness is MBC5-SRAM-direct for M0.** `HarnessChannel` does **not** mutate the emulated MBC SRAM bank register and does **not** call `GameBoy::write` for harness operations. It accesses cartridge RAM directly by `(sram_bank, addr_in_bank)` through the `cartridge.ram: Vec<u8>` backing store (offset = `sram_bank * 0x2000 + (addr - 0xA000)`). The `HarnessMemory` trait exposes `read_sram_bank(bank, addr) / write_sram_bank(bank, addr, value)` instead of `switch_sram_bank` + `peek` / `poke`.

10. **`bus_read` / `bus_write` wording is "adapter-synthesized CPU-bus operations".** They route through gameroy's public `read` / `write` methods, advance the clock by one CPU memory-access quantum, and record adapter-originated trace entries with `TraceOrigin::HostBus`. They are *not* identical to the interpreter's private `gb_read` / `gb_write`; F-A7 does not claim bit-exact equivalence.

The spike at `gbf-emu/tests/gameroy_api_spike.rs` either confirms each of (1)–(10) against the pinned rev or amends this list (and the affected sections below) in the implementation PR.

## 4. `adapter.rs` and `primitives.rs` (T-A7.1, T-A7.2, `bd-1t5d` + `bd-1aql`)

**Beads**: `bd-1t5d` (P0, scaffold) and `bd-1aql` (P0, core API). **Reference**: planv0 line 313 (library API shape).

### 4.1 Why `Emulator` is the keystone type

`Emulator` is the only type consumers carry around as a value. Every other module's outputs are read off it (`regs()`, `framebuffer()`) or installed into it (`add_pc_trap()`, `attach_harness()`). If `Emulator` were the wrong shape — too narrow, too thread-y, too callback-heavy — every consumer would either re-wrap gameroy themselves or carry a bag of side-channels. Both outcomes destroy the single-substrate property.

The shape this RFC commits to is in §3.3. The fields are private; mutation happens through verbs:

```rust
pub struct EmulatorConfig {
    pub policy: DeterminismPolicy,
    pub boot_mode: BootMode,
    pub trace_capacity: usize,
    pub trace_drop_policy: TraceDropPolicy,
}

impl Default for EmulatorConfig { /* policy=default; boot_mode=PostBootDmg; trace_capacity=4096; drop=DropOldest */ }

impl Emulator {
    // Construction
    pub fn builder() -> EmulatorBuilder;
    pub fn load_rom(bytes: &[u8], config: EmulatorConfig) -> Result<Self, EmuError>;

    // Execution
    pub fn step(&mut self) -> Result<StepOutcome, EmuError>;
    pub fn run_for(&mut self, budget: CycleBudget) -> Result<RunOutcome, EmuError>;
    pub fn run_until_pc(&mut self, pc: u16, budget: CycleBudget) -> Result<RunOutcome, EmuError>;

    // Register access (validated)
    pub fn regs(&self) -> Regs;
    pub fn set_regs(&mut self, regs: Regs) -> Result<(), EmuError>;

    // Bus access — adapter-synthesized CPU-bus operations. Routes through gameroy's
    // public read/write, advances the clock by one CPU memory-access quantum, and
    // records adapter-originated trace entries with TraceOrigin::HostBus. NOT identical
    // to the interpreter's private gb_read/gb_write — F-A7 does not claim bit-exact
    // equivalence (see §3.7.1 finding 10).
    pub fn bus_read(&mut self, addr: u16) -> Result<u8, EmuError>;
    pub fn bus_write(&mut self, addr: u16, value: u8) -> Result<(), EmuError>;

    // Debugger access — side-effect-free **only** for raw-backed regions (ROM, WRAM,
    // HRAM, SRAM at an explicit bank, and — if the spike confirms direct access — VRAM/OAM).
    // Returns EmuError::DebugMemoryUnsupported { addr } for IO ($FF00..=$FF7F) and any
    // region the pinned gameroy-core revision does not expose for raw access. IO inspection
    // goes through bus_read instead (see §3.7.1 finding 3, Option B).
    pub fn peek(&self, addr: u16) -> Result<u8, EmuError>;
    pub fn poke(&mut self, addr: u16, value: u8) -> Result<(), EmuError>;
    pub fn peek_range(&self, start: u16, len: usize) -> Result<Vec<u8>, EmuError>;

    // Framebuffer & joypad
    pub fn framebuffer(&self) -> Framebuffer;
    pub fn set_joypad(&mut self, frame: JoypadFrame);

    // Snapshot — captures core state only; not traps / cursor / harness
    pub fn snapshot(&self) -> Result<Snapshot, EmuError>;
    pub fn restore(&mut self, snapshot: &Snapshot) -> Result<(), EmuError>;

    // Trap dispatcher
    pub fn traps(&mut self) -> &mut TrapDispatcher;
    pub fn traps_ref(&self) -> &TrapDispatcher;

    // Trace cursor
    pub fn drain_trace(&mut self) -> Vec<NormalizedTraceEvent>;

    // Harness channel — borrow-safe verbs live on Emulator, not on the channel
    pub fn attach_harness(&mut self, slot: HarnessSlot);
    pub fn poll_harness(&mut self) -> Result<Option<HarnessCommand>, EmuError>;
    pub fn write_harness_result(&mut self, result: HarnessResult) -> Result<(), EmuError>;

    // Identity & clocks
    pub fn rom_sha256(&self) -> Hash256;
    pub fn clock_count(&self) -> ClockCycles;
    pub fn m_cycle_count_floor(&self) -> MCycles;
    pub fn policy(&self) -> &DeterminismPolicy;
    pub fn boot_mode(&self) -> BootMode;
    pub fn version_tag(&self) -> EmuVersionTag;
}
```

`Snapshot` captures only gameroy-core execution state plus lineage. It does **not** capture installed traps, pending trace cursor contents, harness channel sequence state, or F-A8 session metadata. `restore` restores gameroy-core state and synchronizes the adapter's view of cycle count from the restored core; existing traps remain installed unless the caller explicitly clears them. F-A8's session-level snapshots are a strict superset that wraps `Snapshot` with the rest of the session state.

### 4.2 `Regs` — the LR35902 register POD

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Flags(u8);

impl Flags {
    pub fn new(masked: u8) -> Self { Self(masked & 0xF0) }
    pub const fn bits(self) -> u8 { self.0 }
    pub const fn z(self) -> bool { (self.0 & 0x80) != 0 }
    pub const fn n(self) -> bool { (self.0 & 0x40) != 0 }
    pub const fn h(self) -> bool { (self.0 & 0x20) != 0 }
    pub const fn c(self) -> bool { (self.0 & 0x10) != 0 }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum ImeSnapshot {
    /// Interrupts disabled.
    Disabled,
    /// Interrupts enabled.
    Enabled,
    /// `EI` has executed; interrupts will be enabled before the next instruction.
    /// gameroy-core models this as `ImeState::ToBeEnable`.
    ToBeEnable,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct Regs {
    pub a: u8,
    pub f: Flags,             // low nibble masked off in constructor
    pub b: u8,
    pub c: u8,
    pub d: u8,
    pub e: u8,
    pub h: u8,
    pub l: u8,
    pub sp: u16,
    pub pc: u16,
    pub ime: ImeSnapshot,     // tri-state IME — must mirror gameroy's ImeState exactly
}
```

`Regs` covers the LR35902 architectural register file plus IME. CPU execution state (HALT / STOP / interpreter "bug" state) is **not** part of `Regs`; HALT and STOP surface only via `StepOutcome::Idle { state: CpuIdleState }` / `RunOutcome::Idle { state }`. `set_regs` therefore cannot put the emulator into HALT or STOP — that would require a different, currently-unwanted, side door.

`ImeSnapshot` is tri-state because gameroy-core's `ImeState` is tri-state: a `bool` would silently lose the "EI executed; enable on next instruction" state, which is observable around interrupts and HALT (see §3.7.1 finding 2). The conversion to/from gameroy-core's `ImeState` is total in both directions.

The `Flags` newtype prevents `set_regs` from installing impossible flag-register states (low nibble must be zero on real hardware). `set_regs` returns `Result<(), EmuError>` because gameroy-core may also reject some register configurations (e.g., `pc` outside valid bus space depending on the spike's findings).

The conversion to/from gameroy-core's internal register type is a one-shot in `adapter.rs`. F-A7 does not expose gameroy-core's type; consumers see `Regs`. This decouples F-A7's API from gameroy-core's internal field naming and lets us bump the pin without rippling through every consumer.

`Regs` is `serde::Serialize + Deserialize` so test assertions can `assert_eq!` against JSON snapshots.

### 4.3 `RunOutcome`, `StepOutcome`, and `CycleBudget`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ClockCycles(pub u64);

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct MCycles(pub u64);

impl From<MCycles> for ClockCycles {
    fn from(v: MCycles) -> Self { ClockCycles(v.0 * 4) }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum CycleBudget {
    Clock(ClockCycles),
    Machine(MCycles),
}

impl CycleBudget {
    pub fn as_clock_cycles(self) -> ClockCycles {
        match self {
            Self::Clock(c) => c,
            Self::Machine(m) => m.into(),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum CpuIdleState {
    /// CPU executed HALT and is waiting for an interrupt.
    Halt,
    /// CPU executed STOP.
    Stop,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum StepOutcome {
    Stepped { cycles: ClockCycles },
    TrapHit { trap_id: BreakpointId, kind: TrapKind, cycles: ClockCycles },
    Idle { state: CpuIdleState, cycles: ClockCycles },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum RunOutcome {
    /// Normal outcome of `run_for(budget)`: the budget elapsed without halting on a trap or idle.
    /// This is *not* an error; the consumer decides what to do.
    BudgetElapsed { observed: ClockCycles, requested: ClockCycles },
    TrapHit { trap_id: BreakpointId, kind: TrapKind, observed: ClockCycles },
    Idle { state: CpuIdleState, observed: ClockCycles },
}
```

`StepOutcome::Stepped::cycles` is the clock-cycle cost of the just-executed instruction (1..=24 clock = 1..=6 M-cycles). `RunOutcome::BudgetElapsed` is the load-bearing rename: budget exhaustion is a normal outcome of `run_for`, not an `EmuError`. (An earlier draft listed `RunBudgetExhausted` in `EmuError`; that contradicted the `RunOutcome` shape and is removed.) `Idle` distinguishes HALT from STOP — they have different semantics on real hardware and the runtime nucleus cares about the difference. HALT is *not* terminal: a subsequent `step` / `run_for` may exit HALT when an interrupt fires; only STOP is terminal under M0 (and M0 does not emit STOP).

A strict `expect_pc_before_budget(pc, budget)` helper is **out of scope** for F-A7. Consumers that want "must reach this PC or fail" call `run_until_pc(pc, budget)?` and `match` on the outcome:

```rust
match emu.run_until_pc(0x0150, budget)? {
    RunOutcome::TrapHit { .. } => { /* reached */ }
    RunOutcome::BudgetElapsed { observed, requested } => {
        return Err(MyConsumerError::DidNotReachPc { pc: 0x0150, observed, requested });
    }
    RunOutcome::Idle { state, .. } => return Err(MyConsumerError::IdleBeforePc { state }),
}
```

`RunOutcome` does not have a `Faulted` variant because faults in F-A7's vocabulary mean *runtime nucleus faults*, which are F-A5 and F-D5's territory. From `gbf-emu`'s perspective, a "fault" is just a memory write at the fault-flag address; the consumer's trap dispatcher catches it.

### 4.4 `Framebuffer`

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Framebuffer {
    pixels: [u8; 160 * 144],   // each byte in 0..=3 (DMG palette index)
}

impl Framebuffer {
    pub const WIDTH: usize = 160;
    pub const HEIGHT: usize = 144;
    pub fn pixel(&self, x: usize, y: usize) -> u8;
    pub fn as_bytes(&self) -> &[u8; 160 * 144];
    pub fn dmg_palette() -> [Color; 4];
}
```

`Framebuffer` is the DMG four-palette-index form. CGB's RGB palette is non-goal in M0; a follow-up bead adds a `cgb_palette()` accessor when CGB lands.

The `Eq + PartialEq` impl is byte-equality, which is the substrate the determinism integration test needs.

### 4.5 `JoypadFrame`

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Default)]
pub struct JoypadFrame {
    /// Active-high view: bit set ⇒ button pressed.
    pub bits: u8,
}

impl JoypadFrame {
    pub fn pressed(button: gbf_hw::joypad::Button) -> Self;
    pub fn with(self, button: gbf_hw::joypad::Button) -> Self;
    pub fn is_pressed(&self, button: gbf_hw::joypad::Button) -> bool;
}
```

`JoypadFrame` mirrors `gbf_hw::joypad::ButtonState` (active-high), not the JOYP register's active-low encoding. The conversion to gameroy's internal joypad representation happens in `adapter.rs::set_joypad`. Consumers never see the active-low form.

### 4.6 `Snapshot` and `EmuVersionTag`

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub blob: Vec<u8>,                      // gameroy-core save state bytes (deterministic header timestamp)
    pub lineage: SnapshotLineage,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct SnapshotLineage {
    pub rom_sha256: Hash256,
    pub boot: BootModeLineage,              // gameroy-core's save state omits the boot-ROM bytes; we carry the fingerprint here (see §3.7.1 finding 8)
    pub policy_fingerprint: Hash256,
    pub emu_version: EmuVersionTag,
    pub cycle_count: ClockCycles,           // informational; not checked on restore
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum BootModeLineage {
    PostBootDmg,
    BootRom { sha256: Hash256 },            // hash of the BootRomImage.bytes used at construction
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct EmuVersionTag {
    pub gameroy_package: &'static str,      // "gameroy-core"
    pub gameroy_semver: SemVer,             // package version, if available; not load-bearing alone
    pub gameroy_git_rev: GitSha,            // **the** load-bearing pin; injected at build time
    pub gbf_emu_version: SemVer,            // this crate's version
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct GitSha(pub [u8; 20]);            // raw 20-byte SHA-1 digest
```

`Snapshot::lineage` is the load-bearing identity: `Emulator::restore(&snapshot)` rejects with `EmuError::SnapshotRomMismatch` / `SnapshotBootMismatch` / `SnapshotPolicyMismatch` / `SnapshotEmuVersionMismatch` if any of the lineage fields disagree with the current `Emulator`. **`SnapshotEmuVersionMismatch` is keyed on `gameroy_git_rev`, not on `gameroy_semver`.** A published version can stay constant while source moves; only the git revision is a faithful identity for snapshot bytes. **`SnapshotBootMismatch` exists because `gameroy-core`'s save state does not include the boot-ROM bytes themselves** (per §3.7.1 finding 8): a snapshot taken under one boot ROM is under-specified if restored under a different one, so F-A7 carries the boot fingerprint in lineage.

The `cycle_count` field is informational — it is *not* checked on restore (that would forbid restoring to an earlier cycle, which is the whole point of snapshots).

The `gameroy_git_rev` field is injected by `build.rs` from cargo metadata (`cargo metadata --format-version 1`, looking up the `gameroy-core` package's `source` field which encodes the git rev for git dependencies) and embedded as a `const` in the generated code. F-A7 does **not** use `env!("CARGO_PKG_VERSION")` for dependency identity; that env var names the *current* crate (`gbf-emu`), not the dependency.

`Snapshot` captures only gameroy-core execution state. It does not capture installed traps, pending trace cursor contents, harness channel sequence state, or F-A8 session metadata — those are session-level concerns layered on top in F-A8. `Emulator::snapshot()` returns `Result<Snapshot, EmuError>` because gameroy-core's save-state path can fail; the deterministic save-state metadata timestamp (`FIXED_SAVE_STATE_UNIX_MS`) is supplied by F-A7 to make the resulting bytes byte-stable.

### 4.7 Tests

```bash
cargo test -p gbf-emu -- adapter::load_rom_round_trip            # bytes in → SHA-256 stored → restore matches
cargo test -p gbf-emu -- adapter::step_advances_pc                # step bumps PC by the encoded length
cargo test -p gbf-emu -- adapter::run_for_budget_elapsed          # exhausting budget returns RunOutcome::BudgetElapsed
cargo test -p gbf-emu -- adapter::run_until_pc_fires              # PC trap fires once before the instruction at addr
cargo test -p gbf-emu -- adapter::run_until_pc_returns_immediately_when_already_there
cargo test -p gbf-emu -- adapter::regs_round_trip                 # set_regs → regs returns equal value (within Flags mask)
cargo test -p gbf-emu -- adapter::set_regs_masks_low_flag_nibble  # Flags::new masks 0x0F bits to zero
cargo test -p gbf-emu -- adapter::bus_read_write_round_trip       # bus_write → bus_read returns the same byte (within RAM ranges)
cargo test -p gbf-emu -- adapter::peek_does_not_advance_clock     # peek leaves clock_count unchanged
cargo test -p gbf-emu -- adapter::peek_does_not_emit_trace_events # peek does not push a NormalizedTraceEvent
cargo test -p gbf-emu -- adapter::poke_does_not_trigger_guest_traps
cargo test -p gbf-emu -- adapter::peek_range_consistent           # peek_range equals byte-by-byte peek
cargo test -p gbf-emu -- adapter::framebuffer_byte_stable          # framebuffer is byte-equal across reruns
cargo test -p gbf-emu -- adapter::joypad_round_trip                # set_joypad → run → input is observed at JOYP_REGISTER
cargo test -p gbf-emu -- adapter::clock_count_advances_monotonically
cargo test -p gbf-emu -- adapter::cycle_budget_clock_machine_equivalence  # CycleBudget::Machine(M) == Clock(M*4)
cargo test -p gbf-emu -- primitives::regs_serde_round_trip         # JSON round-trip
cargo test -p gbf-emu -- primitives::flags_low_nibble_masked
cargo test -p gbf-emu -- primitives::framebuffer_eq_byte_stable    # PartialEq is byte-equality
cargo test -p gbf-emu -- primitives::snapshot_lineage_serde         # SnapshotLineage round-trips through serde
cargo test -p gbf-emu -- primitives::emu_version_tag_records_git_rev # gameroy_git_rev equals Cargo.lock pin
```

## 5. `determinism.rs` (T-A7.3, `bd-10y1`)

**Bead**: `bd-10y1` (P0). **Reference**: planv0 line 2729.

### 5.1 Why `DeterminismPolicy` is a value, not a flag-bag

The naive shape would be `pub struct DeterminismPolicy { pub fixed_rtc: bool, pub rng_seed: Option<u64>, pub audio_enabled: bool }`. That is wrong for several reasons:

1. **Defaults are silent.** `Default::default()` with three booleans makes "what's locked down by default?" non-obvious. Consumers reading test code would have to consult the `Default` impl to know whether RTC is real-time or pinned.
2. **Opt-outs are flag flips.** A test that says `.fixed_rtc = false` reads as a configuration adjustment, not as "this test deliberately uses real-time RTC because it's a UI smoke test." The verb form `with_real_time_cartridge_rtc()` is self-documenting; the boolean form is not.
3. **`rng_seed` is underspecified.** The Game Boy has no built-in RNG; "RNG seed" silently picks one of several possible meanings (uninitialized RAM, host entropy, save-state metadata) and conflates them. A typed `PowerOnRamPolicy` enum names the actual entropy source.
4. **The save-state metadata timestamp is a separate dimension.** Cartridge RTC, host audio, and uninitialized RAM are emulation-time concerns; save-state metadata is a serialization concern. Fusing them into one `fixed_rtc` bool would conflate three independent decisions.

The shape this RFC commits to:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct DeterminismPolicy {
    cartridge_rtc: CartridgeRtcMode,
    save_state_metadata: SaveStateMetadataMode,
    power_on_ram: PowerOnRamPolicy,
    audio_output: AudioOutputMode,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum CartridgeRtcMode {
    /// Pinned to FIXED_CARTRIDGE_RTC_UNIX_MS. No-op for MBC5; live for MBC3/HuC-3.
    Fixed,
    /// Real-time RTC. Reserved for UI smoke tests against RTC-bearing cartridges.
    RealTime,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum SaveStateMetadataMode {
    /// Pin gameroy-core's save-state header timestamp to FIXED_SAVE_STATE_UNIX_MS.
    Fixed,
    /// Use whatever gameroy-core encodes for "no timestamp", if that itself is deterministic.
    None,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum PowerOnRamPolicy {
    /// gameroy-core's defaults: 0xFF for WRAM/HRAM, 0x00 for cartridge RAM. Deterministic
    /// for the pinned rev; the determinism single-path test is the load-bearing check.
    GameroyDefault,
    /// Fill uninitialized RAM regions with explicit per-region bytes after construction.
    /// The adapter writes through the public mutable backing-store fields the spike confirms;
    /// if any field is private at the pinned rev, the variant returns
    /// EmuError::Determinism { reason: "FixedFill region inaccessible" } at load_rom.
    FixedFill { wram: u8, hram: u8, cartridge_ram: u8 },
    // Seeded { seed: u64 } REMOVED for M0 (see §3.7.1 finding 7) — the Game Boy has no
    // built-in RNG and gameroy-core does not expose a seedable power-on-RAM API. Reintroduce
    // only if the spike proves an upstream hook; otherwise add as a follow-up bead.
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum AudioOutputMode {
    /// No host audio sink attached. APU emulation state evolves normally; only the
    /// host output is suppressed. This is the **only** value supported under the headless
    /// gameroy-core dependency (see §3.7.1 finding 7); the builder's opt-in
    /// `with_audio_output_enabled()` was removed.
    Disabled,
}

pub const FIXED_CARTRIDGE_RTC_UNIX_MS: i64 = 946_684_800_000;
pub const FIXED_SAVE_STATE_UNIX_MS: u64   = 946_684_800_000;

impl Default for DeterminismPolicy {
    fn default() -> Self {
        Self {
            cartridge_rtc: CartridgeRtcMode::Fixed,
            save_state_metadata: SaveStateMetadataMode::Fixed,
            power_on_ram: PowerOnRamPolicy::GameroyDefault,
            audio_output: AudioOutputMode::Disabled,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DeterminismPolicyBuilder {
    cartridge_rtc: Option<CartridgeRtcMode>,
    save_state_metadata: Option<SaveStateMetadataMode>,
    power_on_ram: Option<PowerOnRamPolicy>,
    audio_output: Option<AudioOutputMode>,
}

impl DeterminismPolicyBuilder {
    pub fn new() -> Self;
    pub fn with_real_time_cartridge_rtc(self) -> Self;
    // with_audio_output_enabled() REMOVED — gbf-emu depends on the headless gameroy-core
    // package, which has no host audio sink. Re-enabling would require depending on the
    // GUI/application package. See §3.7.1 finding 7.
    pub fn with_power_on_ram(self, p: PowerOnRamPolicy) -> Self;
    pub fn with_save_state_metadata(self, m: SaveStateMetadataMode) -> Self;
    pub fn build(self) -> DeterminismPolicy;
}

impl DeterminismPolicy {
    pub fn fingerprint(&self) -> Hash256;
    pub fn cartridge_rtc(&self) -> CartridgeRtcMode;
    pub fn save_state_metadata(&self) -> SaveStateMetadataMode;
    pub fn power_on_ram(&self) -> PowerOnRamPolicy;
    pub fn audio_output(&self) -> AudioOutputMode;
}
```

Notes:

- **`CartridgeRtcMode::Fixed` is included in the policy fingerprint for future support but is *applied* only when gameroy-core exposes a cartridge RTC hook.** For M0's MBC5 target it is unobserved (MBC5 has no RTC surface; Pan Docs §"MBC3" describes the RTC, MBC5 does not). Loading an RTC-bearing cartridge (MBC3 with timer, HuC-3) under F-A7 returns `EmuError::Determinism { reason: "cartridge RTC control unavailable" }` unless RTC control is available in the pinned rev (see §3.7.1 finding 7).
- **`SaveStateMetadataMode::Fixed` is the load-bearing piece for byte-equal snapshots.** gameroy-core's `save_state(timestamp, writer)` accepts an optional timestamp; if F-A7 ever passes wall-clock there, two snapshots taken seconds apart will not byte-compare even when execution is fully deterministic. This is *separate* from cartridge RTC — they touch different bytes.
- **`PowerOnRamPolicy` replaces the earlier draft's `rng_seed: u64`.** The Game Boy has no built-in RNG; the determinism question is about uninitialized RAM. `GameroyDefault` is the locked-down default (gameroy-core initializes WRAM/HRAM to `0xFF` and cartridge RAM to `0x00`). `FixedFill { wram, hram, cartridge_ram }` is per-region because the three regions have different gameroy defaults. **`Seeded { seed }` was removed** for M0 — gameroy-core does not expose a seedable power-on-RAM API and the Game Boy has no native randomness source (see §3.7.1 finding 7). Reintroduce as a follow-up bead only if an upstream hook lands.
- **`AudioOutputMode` is single-variant (`Disabled`) for M0** because `gbf-emu` depends on the headless `gameroy-core` package, which has no host audio sink. APU emulation still evolves normally — only host output is suppressed. The previous `Enabled` variant and the builder's `with_audio_output_enabled()` were removed (see §3.7.1 finding 7); to re-enable host audio, F-A7 would need to depend on the GUI/application `gameroy` package, which §10's dependency posture forbids.

`fingerprint()` is the load-bearing method: `Snapshot::lineage.policy_fingerprint` is its output. Two policies with identical fields produce identical fingerprints; one bit different anywhere produces a different one. The hash is SHA-256 over a stable byte serialization of all four fields including any payload bytes (`PowerOnRamPolicy::FixedFill(b)` contributes `b`; `Seeded { seed }` contributes `seed.to_le_bytes()`).

### 5.2 Why every mode is a named variant, not a flag

A previous draft modeled `rtc: Option<i64>` where `Some(unix_ms)` meant "pin to this value" and `None` meant "use real time." That made the locked-down case the "no value" case, which is exactly backwards: the locked-down case is what we want to be obvious. The enum form makes every case first-class and the `Default` impl chooses the locked-down variant.

### 5.3 Tests

```bash
cargo test -p gbf-emu -- determinism::default_is_locked_down       # all four fields are the locked-down variant
cargo test -p gbf-emu -- determinism::fingerprint_stable           # fingerprint of default is byte-stable across runs
cargo test -p gbf-emu -- determinism::fingerprint_distinguishes    # toggling any field changes the fingerprint
cargo test -p gbf-emu -- determinism::builder_round_trip           # builder produces equivalent policy
cargo test -p gbf-emu -- determinism::with_real_time_cartridge_rtc_named  # opt-out is a named verb
cargo test -p gbf-emu -- determinism::with_audio_output_enabled_named
cargo test -p gbf-emu -- determinism::power_on_ram_seeded_distinguishes
cargo test -p gbf-emu -- determinism::save_state_header_timestamp_fixed  # snapshot bytes are byte-equal across runs
cargo test -p gbf-emu -- determinism::single_path                  # same ROM + same policy → byte-equal trace + final state
```

The `single_path` test is the load-bearing one. It runs `tiny_rom.gb` twice with `DeterminismPolicy::default()` for a fixed `ClockCycles` budget and asserts:

```rust
let (fb1, regs1, mem1) = run_full(&rom_bytes, &policy);
let (fb2, regs2, mem2) = run_full(&rom_bytes, &policy);
assert_eq!(fb1, fb2);
assert_eq!(regs1, regs2);
assert_eq!(mem1, mem2);
```

If this test ever fails, the determinism contract has broken — and *every* consumer (`gbf-test`, `gbf-bench`, `gbf-debug`, `gbf-codegen`) is downstream of that breakage. The test runs in pre-commit. The `save_state_header_timestamp_fixed` test is the sibling that proves snapshot bytes themselves are stable (necessary for F-A8 session lineage to round-trip cleanly).

## 6. `trap.rs` (T-A7.4, `bd-19as`)

**Bead**: `bd-19as` (P0). **Reference**: planv0 line 313 ("registry-driven trap dispatcher (PC traps, memory-access traps with read/write/rw kinds, optional persisted predicates)").

### 6.1 Why a registry, not a callback per consumer

The naive shape would be `Emulator::on_pc(addr, callback)` with a stored `Vec<Box<dyn Fn>>`. That is wrong because:

1. Callbacks are opaque — the dispatcher cannot answer "what traps are currently armed?" without iterating, and the consumer cannot inspect a session file's traps without a runtime.
2. Callbacks cannot be persisted across CLI invocations.
3. Callbacks make the cross-cutting "match this address against a range" logic awkward.

The registry shape:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BreakpointId(pub u32);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum TrapKind {
    Pc { addr: u16 },
    MemRead { range: AddressRange },
    MemWrite { range: AddressRange },
    MemRw { range: AddressRange },
}

/// Constructor-validated half-closed-friendly range. Fields are private; `new` rejects
/// `start > end_inclusive`.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct AddressRange {
    start: u16,
    end_inclusive: u16,
}

impl AddressRange {
    pub fn new(start: u16, end_inclusive: u16) -> Result<Self, AddressRangeError> {
        if start > end_inclusive {
            return Err(AddressRangeError::StartAfterEnd { start, end_inclusive });
        }
        Ok(Self { start, end_inclusive })
    }
    pub const fn start(self) -> u16 { self.start }
    pub const fn end_inclusive(self) -> u16 { self.end_inclusive }
    pub const fn contains(self, addr: u16) -> bool {
        self.start <= addr && addr <= self.end_inclusive
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TrapAction {
    HaltAndReport,
    Continue,                  // log only; do not halt the run
}

pub enum Predicate {
    Always,
    /// In-script use; **single-threaded** by design. Not serializable.
    Closure(Box<dyn FnMut(&TrapContext<'_>) -> Result<bool, TrapPredicateError> + 'static>),
    /// Cross-invocation persistence. Opaque to gbf-emu; F-A8's rquickjs host compiles
    /// it into a `Closure` before the dispatcher runs.
    Source(String),
}

/// Snapshot of execution state passed to a `Closure` predicate. Predicates have no
/// path to traps, harness, trace cursor, or cycle counters; the only memory peek
/// available to them is side-effect-free.
pub struct TrapContext<'a> {
    pub regs: Regs,
    pub pc: u16,
    pub access: Option<MemoryAccess>,
    pub cycle: ClockCycles,
    pub view: EmuReadOnlyView<'a>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct MemoryAccess {
    pub addr: u16,
    pub value: u8,
    pub kind: MemoryAccessKind,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum MemoryAccessKind {
    /// Opcode/operand fetch by the CPU. Under the io_trace backend these are
    /// indistinguishable from data reads at the wire level, so MemRead range traps
    /// fire on instruction fetches inside the range. Tests that want to filter
    /// fetches must filter in the predicate.
    InstrFetch,
    /// Data read (LD A,(HL) and friends). Under io_trace, see InstrFetch caveat.
    DataRead,
    /// Memory write.
    Write,
}

/// Side-effect-free read view backed by `Emulator::peek`. Predicates use this
/// instead of `&Emulator` so they cannot perturb timers, traps, or cursors.
pub struct EmuReadOnlyView<'a> { /* opaque; private fields */ _phantom: core::marker::PhantomData<&'a ()> }

impl EmuReadOnlyView<'_> {
    pub fn peek(&self, addr: u16) -> u8;
    pub fn peek_range(&self, start: u16, len: usize) -> Vec<u8>;
    pub fn regs(&self) -> Regs;
}

/// The serializable persistence shape consumed by F-A8 sessions. `Predicate` itself
/// is not `Serialize`/`Deserialize`.
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

#[derive(Clone, Debug)]
pub struct TrapListEntry<'a> {
    pub id: BreakpointId,
    pub kind: &'a TrapKind,
    pub action: &'a TrapAction,
    /// `None` when the predicate is closure-only and cannot be persisted.
    pub persistable_predicate: Option<&'a str>,
}

pub struct TrapDispatcher { /* private */ }

impl TrapDispatcher {
    pub fn add_pc(&mut self, addr: u16, predicate: Predicate, action: TrapAction) -> BreakpointId;
    pub fn add_mem_read(&mut self, range: AddressRange, predicate: Predicate, action: TrapAction) -> BreakpointId;
    pub fn add_mem_write(&mut self, range: AddressRange, predicate: Predicate, action: TrapAction) -> BreakpointId;
    pub fn add_mem_rw(&mut self, range: AddressRange, predicate: Predicate, action: TrapAction) -> BreakpointId;
    pub fn remove(&mut self, id: BreakpointId) -> bool;
    pub fn remove_entry(&mut self, id: BreakpointId) -> Option<RemovedTrap>;
    pub fn list(&self) -> impl Iterator<Item = TrapListEntry<'_>>;
    pub fn export_persistable_specs(&self) -> Result<Vec<TrapSpec>, TrapPersistenceError>;
    pub fn clear(&mut self);
}
```

`Predicate` does *not* derive `Serialize`/`Deserialize` because the `Closure` variant cannot. F-A8 serializes a session by walking `export_persistable_specs()`, which:

- emits one `TrapSpec` per entry whose predicate is `Always` or `Source`,
- returns `TrapPersistenceError::ClosureOnly { id }` if any entry is `Closure`-only without a recorded source form.

`PredicateSpec::Source` carries the JS source text verbatim; F-A7 does not parse it, evaluate it, or validate that it's well-formed. F-A8's rquickjs host compiles each `Source` into a `Closure` (with the appropriate `TrapContext` binding) before the dispatcher runs; the `Closure` shadow is held alongside the original `Source` so a later `export_persistable_specs()` round-trips.

`Closure` predicates are `FnMut` so they can update internal counters (e.g., "fire on the third hit") and return `Result<bool, TrapPredicateError>` so a malformed predicate surfaces as a typed `EmuError::TrapPredicate` rather than silently halting. They are **not** `Send + Sync`: the entire `Emulator` is single-threaded, and requiring `Send + Sync` would make F-A8's rquickjs binding harder (the JS runtime handle is typically `!Sync`).

`TrapDispatcher::remove` returns `bool` (`true` if an entry was removed). The richer `remove_entry` returns `Option<RemovedTrap>` carrying public-only fields — the previous draft returned a private `TrapEntry`, which is not a valid public API shape.

### 6.2 Dispatch phase

The trap dispatcher is wired into whichever access-observation mechanism the §3.7 adapter spike proves. Per §3.7.1 finding 4, M0 commits to an `io_trace`-backed substrate: there is no public per-memory-access callback in `gameroy-core` that can stop the CPU mid-instruction. PC traps remain pre-instruction; memory traps are **post-instruction** traps under the `io_trace` backend. Phase ordering inside one `step`:

1. **Before executing the instruction at the current `pc`**, the dispatcher checks PC traps (`TrapKind::Pc { addr }`). This matches what debuggers expect: "break before executing the opcode at `addr`." `run_until_pc(pc, budget)` returns `Ok(RunOutcome::TrapHit { ... })` immediately if the emulator is already positioned at `pc` (zero-cycle case). PC traps are the only kind whose state report is **pre-instruction**.
2. **The instruction executes to completion**; gameroy-core records every memory access into the `io_trace` buffer.
3. **After the instruction executes**, the adapter drains the `io_trace` buffer (per §3.7.1 finding 5: drained per-`interpret_op`, never accumulated across multiple instructions). The dispatcher walks the drained accesses in order; the first access matching any memory trap fires that trap. The reported emulator state is **post-instruction** — the CPU has already advanced past the access. The cycle for the trap report is the access's reconstructed `ClockCycles` (from the step's `start_clock`/`end_clock` plus per-access ordering), not the post-instruction `clock_count`.
4. **Then** the trace cursor records the drained accesses as `NormalizedTraceEvent`s and the cycle counters' canonical view updates.

Exact pre-access memory traps are an explicit non-goal for M0 (see §3.7.1 finding 4 and resolved question 32). They require an upstream-instrumented `gb_read` / `gb_write` callback in `gameroy-core` and are deferred to a follow-up bead. Tests that want exact pre-access semantics must use PC traps at the call site instead.

Predicate evaluation order at a match:

- `Always` evaluates to `true` immediately.
- `Closure` is invoked with a fresh `TrapContext` snapshot; it returns `Ok(true)` to fire, `Ok(false)` to skip, `Err(TrapPredicateError)` to surface as `EmuError::TrapPredicate`.
- `Source` raises `EmuError::TrapPredicate(TrapPredicateError::SourceRequiresEvaluator)` if F-A7's dispatcher hits it directly. F-A8's wrapper compiles `Source` into `Closure` before handing the dispatcher off to F-A7's runtime, so a session-resumed dispatcher should never see a bare `Source` predicate at dispatch time.

On `predicate == Ok(true)`:

- `TrapAction::HaltAndReport` returns `StepOutcome::TrapHit { trap_id, kind, cycles }` from `step` / `RunOutcome::TrapHit { ... }` from `run_for` / `run_until_pc`.
- `TrapAction::Continue` records `NormalizedTraceEvent::TrapHit { trap_id, kind, cycle }` (an **F-A7-local** variant — F-A7 does not invent an F-A3 `TraceEvent::TrapHit`) and the run continues.

The dispatch is O(N) over the number of installed traps. F-A7 does not optimize with an interval tree — N is small (M0 expects single-digit traps in any test) and the per-step cost is dominated by gameroy-core itself.

### 6.3 Tests

```bash
cargo test -p gbf-emu -- trap::pc_breakpoint_fires_before_instruction  # PC trap fires before the instruction at addr
cargo test -p gbf-emu -- trap::run_until_pc_returns_immediately_when_at_pc
cargo test -p gbf-emu -- trap::mem_watchpoint_read_reports_first_matching_access_after_step   # MemRead fires post-instruction at the first matching read in the io_trace drain
cargo test -p gbf-emu -- trap::mem_watchpoint_write_reports_first_matching_access_after_step  # MemWrite fires post-instruction at the first matching write
cargo test -p gbf-emu -- trap::mem_watchpoint_rw_reports_first_matching_access_after_step     # MemRw fires post-instruction on either kind
cargo test -p gbf-emu -- trap::mem_watchpoint_read_includes_instr_fetches_under_io_trace      # documents the io_trace fetch-vs-data fold
cargo test -p gbf-emu -- trap::mem_watchpoint_state_is_post_instruction                       # TrapContext.regs reflects the post-instruction state
cargo test -p gbf-emu -- trap::predicate_always_fires               # Always predicate fires every cycle the trap matches
cargo test -p gbf-emu -- trap::predicate_closure_reads_trap_context # Closure sees regs/pc/access through TrapContext
cargo test -p gbf-emu -- trap::predicate_closure_cannot_perturb_state  # peek through view does not advance clock
cargo test -p gbf-emu -- trap::predicate_closure_fnmut_counter      # Closure can mutate captured counters
cargo test -p gbf-emu -- trap::predicate_closure_error_surfaces_as_emu_error
cargo test -p gbf-emu -- trap::predicate_source_returns_typed_error  # Source predicate hits TrapPredicateError::SourceRequiresEvaluator
cargo test -p gbf-emu -- trap::remove_returns_bool                   # remove(id) returns true/false
cargo test -p gbf-emu -- trap::remove_entry_returns_public_struct    # remove_entry → Option<RemovedTrap>
cargo test -p gbf-emu -- trap::halt_vs_continue_actions              # HaltAndReport halts; Continue logs
cargo test -p gbf-emu -- trap::continue_action_logs_normalized_trap_hit  # NormalizedTraceEvent::TrapHit (not F-A3)
cargo test -p gbf-emu -- trap::list_round_trip                       # list() reflects add/remove
cargo test -p gbf-emu -- trap::address_range_rejects_inverted        # AddressRange::new(0x10, 0x00) is Err
cargo test -p gbf-emu -- trap::export_persistable_specs_skips_closures
cargo test -p gbf-emu -- trap::trap_spec_serde_round_trip
cargo test -p gbf-emu -- trap::serde_kind_action_round_trip          # TrapKind/TrapAction round-trip JSON
```

## 7. `trace_ring.rs` (T-A7.5, `bd-14yy`)

**Bead**: `bd-14yy` (P1). **Reference**: planv0 line 313, line 207.

### 7.1 Why a *normalized* event format

The naive shape would be "expose gameroy's raw memory-access hooks to consumers." That is wrong because:

1. Whatever access-observation mechanism the §3.7 spike proves will surface accesses at the *cycle* level (a `tick`-driven `io_trace` buffer, an interpreter instrumentation point, or an F-A7-local wrapper); consumers want the *event* level. A 16-bit memory write on LR35902 fires two cycle-level accesses (low byte + high byte); the canonical event is one `MemoryWrite { addr, value: u16, ... }`. F-A7 normalizes.
2. Different consumers want different framings. F-D3's trace ring is per-event; `gbf-debug`'s trace ring is per-event; `gbf-bench`'s drift report is per-event aggregated. None of them want raw cycle-level callbacks.
3. The ROM-bank-switch and SRAM-bank-switch events are *derived*: they happen when the consumer writes to the MBC5 BANK1/BANK2/RAMB registers (`gbf_hw::mbc5::{BANK1, BANK2, RAMB}`). Detecting them requires correlating two writes (BANK1 + BANK2 for a 9-bit ROM bank). F-A7 normalizes once; consumers get the bank-switch event directly.

The shape this RFC commits to:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NormalizedTraceEvent {
    /// Memory write at address with value, fully tagged with bank context and origin.
    MemoryWrite {
        addr: u16,
        value: u8,
        region: gbf_hw::memory::MemoryRegion,
        bank: BankSnapshot,
        origin: TraceOrigin,
        cycle: ClockCycles,
    },
    /// ROM bank switch detected after a BANK1+BANK2 (or BANK2-only when the high bit changes) write.
    RomBankSwitch {
        from: u16,
        to: u16,
        source: BankSwitchSource,
        cycle: ClockCycles,
    },
    /// SRAM bank switch via RAMB write.
    SramBankSwitch {
        from: u8,
        to: u8,
        cycle: ClockCycles,
    },
    /// IO write into the IO region ($FF00..=$FF7F).
    IoWrite {
        reg: u16,
        value: u8,
        cycle: ClockCycles,
    },
    /// Trap-hit log entry recorded when a trap fires with `TrapAction::Continue`. F-A7-local;
    /// **not** an F-A3 `TraceEvent` variant. F-A7 does not invent F-A3 wire-format types.
    TrapHit {
        trap_id: BreakpointId,
        kind: TrapKind,
        cycle: ClockCycles,
    },
    /// Typed event from the F-A3 channel passthrough. The runtime nucleus emits these via
    /// the trace ring; F-A7 normalizes them into the same stream consumers see.
    Typed(gbf_abi::trace::TraceEvent),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum BankSwitchSource {
    Bank1Write { value: u8 },
    Bank2Write { value: u8 },
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TraceOrigin {
    /// Guest CPU executed an instruction that performed this access. The default
    /// for trace events drained from gameroy-core's io_trace buffer per-step.
    GuestCpu,
    /// DMA-driven access (OAM DMA, future CGB VRAM DMA).
    Dma,
    /// Host-invoked side-effecting bus access via Emulator::bus_read / bus_write.
    /// These advance the clock and route through gameroy's public read/write, but
    /// they are NOT guest CPU execution. See §3.7.1 finding 6 + finding 10.
    HostBus,
    /// Host-originated debugger poke or harness write. These are recorded only when
    /// debug auditing is explicitly enabled; they do not advance the clock and do
    /// not represent guest execution.
    HostPoke,
}

/// Bank context at the moment of an event. Consumers that compare traces across bank
/// switches need this so a memory write to `$D000` (WRAM bank 1 on CGB) is not collapsed
/// with a write to `$D000` in WRAM bank 2.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct BankSnapshot {
    pub rom: u16,    // 9-bit MBC5 ROM bank (0..=511)
    pub sram: u8,    // SRAM bank
    pub wramx: u8,   // CGB WRAM bank (1..=7); 1 on DMG
    pub vram: u8,    // CGB VRAM bank (0..=1); 0 on DMG
}
```

The `cycle` field in every variant is the **clock-cycle** count at which the event fired (typed `ClockCycles`). Consumers that aggregate events use it as the time axis. M-cycle views are obtained via `MCycles::from(cycle)` (`n / 4`).

`gameroy-core`'s `io_trace` records each access with a packed `u8` that carries `(kind | ((clock_count & !3) >> 1))`, not an absolute `u64`. Per §3.7.1 finding 5, the adapter drains `io_trace` once per `interpret_op` invocation and reconstructs each event's `ClockCycles` from the step's `start_clock` / `end_clock` plus per-access ordering. The trace ring **never** consumes an `io_trace` buffer accumulated across multiple instructions; doing so would lose unique cycle resolution and corrupt the time axis.

`MemoryRegion` comes from F-A2's `gbf_hw::memory::classify(addr)`. The classification happens once at normalization time, not every time a consumer reads the event; this saves the consumer the import.

The `Typed(TraceEvent)` variant is a passthrough for F-A3's typed event channel. The runtime nucleus emits typed `TraceEvent`s via a designated SRAM region (F-D3's wire format); F-A7 reads that region, parses it via `gbf_abi::trace::TraceEvent::from_le_bytes`, and surfaces the event in the same `NormalizedTraceEvent` stream consumers see. The reason: consumers should not need two parallel APIs ("raw memory writes" and "typed events") — they're both *trace events*.

Trap-hit logging uses `NormalizedTraceEvent::TrapHit`, **not** an F-A3 `TraceEvent::TrapHit`. F-A7 owns its own normalized event taxonomy; F-A3 owns the typed-channel wire format that the runtime nucleus emits. The two coexist; F-A7 does not extend F-A3's enum.

**Duplicate suppression.** The `TraceCursor` preserves every guest-visible event exactly once. Two identical writes to the same address in consecutive cycles are real and must produce two events. Any backend-specific duplicate suppression (e.g., to mask gameroy-core's `io_trace` buffer firing the same byte twice for instrumentation reasons) must be backed by a fixture in `tests/trace_ring.rs` proving the duplicate is instrumentation noise rather than a real second guest access.

### 7.2 `TraceCursor`

```rust
pub struct TraceCursor { /* private */ }

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndError,
}

impl TraceCursor {
    pub fn new(capacity: usize, drop_policy: TraceDropPolicy) -> Self;
    pub(crate) fn record(&mut self, event: NormalizedTraceEvent) -> Result<(), TraceCursorError>;
    pub fn drain(&mut self) -> Vec<NormalizedTraceEvent>;
    pub fn len(&self) -> usize;
    pub fn capacity(&self) -> usize;
}
```

`record` is `pub(crate)` because only `adapter.rs` is allowed to push events; consumers see only `drain()`. Internal cursor state tracks the MBC5 BANK1/BANK2/RAMB shadow registers (and CGB WRAM/VRAM bank selects, for future use) so the cursor can:

- emit `RomBankSwitch { from, to, source }` when a BANK1+BANK2 (or BANK2-only) write changes the resulting 9-bit bank number;
- attach a fully-populated `BankSnapshot` to every `MemoryWrite` event, regardless of whether the consumer is currently looking at the bank-switch-derived events.

The `EmulatorConfig` (§4.1) supplies `trace_capacity` and `trace_drop_policy` at `load_rom` time; consumers that want a different ring size do so through the config, not through a separate setter.

`TraceDropPolicy::HaltAndError` makes the next `step` / `run_for` / `run_until_pc` return `EmuError::TraceCapacityExceeded` instead of dropping. Useful for tests that assert "this run should fit in N events."

### 7.3 Tests

```bash
cargo test -p gbf-emu -- trace_ring::memory_write_event                  # writing 0x55 to $C000 emits MemoryWrite{...}
cargo test -p gbf-emu -- trace_ring::memory_write_carries_bank_snapshot  # rom/sram/wramx/vram banks all populated
cargo test -p gbf-emu -- trace_ring::memory_write_origin_guest_cpu       # default origin is GuestCpu
cargo test -p gbf-emu -- trace_ring::bus_write_origin_host_bus           # bus_write tags the event TraceOrigin::HostBus
cargo test -p gbf-emu -- trace_ring::poke_does_not_emit_guest_event      # host poke does not produce GuestCpu
cargo test -p gbf-emu -- trace_ring::poke_origin_host_poke_when_audited  # explicit-audit mode tags pokes HostPoke
cargo test -p gbf-emu -- trace_ring::io_trace_drained_per_instruction    # io_trace cleared after each interpret_op
cargo test -p gbf-emu -- trace_ring::cycle_reconstructed_from_step_clocks # ClockCycles derived from start_clock/end_clock + access order
cargo test -p gbf-emu -- trace_ring::two_identical_consecutive_guest_writes_are_two_events
cargo test -p gbf-emu -- trace_ring::rom_bank_switch_low_byte             # BANK1=0x05 with BANK2=0 emits RomBankSwitch{from:1,to:5}
cargo test -p gbf-emu -- trace_ring::rom_bank_switch_high_bit             # BANK2=1 with BANK1=0 emits RomBankSwitch{from:1,to:256}
cargo test -p gbf-emu -- trace_ring::sram_bank_switch                     # RAMB=2 emits SramBankSwitch{from:0,to:2}
cargo test -p gbf-emu -- trace_ring::io_write_event                       # writing $FF40 emits IoWrite{reg:0xFF40,...}
cargo test -p gbf-emu -- trace_ring::trap_hit_event_is_local              # TrapHit is NormalizedTraceEvent::TrapHit, not Typed(...)
cargo test -p gbf-emu -- trace_ring::typed_passthrough                    # F-A3 Typed events round-trip
cargo test -p gbf-emu -- trace_ring::drop_oldest_under_pressure           # capacity overflow drops oldest
cargo test -p gbf-emu -- trace_ring::halt_and_error_on_overflow           # HaltAndError raises EmuError::TraceCapacityExceeded
cargo test -p gbf-emu -- trace_ring::region_classification_correct        # MemoryWrite.region == gbf_hw::memory::classify(addr)
cargo test -p gbf-emu -- trace_ring::cycle_field_is_clock_cycles          # cycle: ClockCycles, not raw u64
cargo test -p gbf-emu -- trace_ring::serde_round_trip                      # JSON round-trip for every variant
```

## 8. `harness.rs` (T-A7.6, `bd-16z8`)

**Bead**: `bd-16z8` (P1). **Reference**: planv0 line 313 ("harness-mode execution path that tests consume"); F-A3 §3.3 (`HarnessCommandBlock`/`HarnessResultBlock` layouts).

### 8.1 Why `HarnessChannel` is plumbing only

The naive shape would be `Emulator::run_harness_op(op: HarnessOp) -> HarnessResult` with a giant `match` on the opcode. That is wrong because:

1. **`HarnessOp::RunUntilCheckpoint(SemanticCheckpointId)` requires a `SemanticCheckpointSchema`** to resolve the id to a PC address. The schema is build-specific (it's emitted by `gbf-codegen` at build time and consumed by F-D2 at run time). `gbf-emu` has no notion of "the build"; it has only a ROM. Asking F-A7 to dispatch `RunUntilCheckpoint` would force F-A7 to depend on `gbf-artifact` or `gbf-codegen`, which destroys the dependency direction.
2. **`HarnessOp::DumpArena(ArenaId)` requires an `ArenaPlan`** to know which memory ranges constitute the arena. Same dependency-direction problem.
3. **`HarnessOp::InjectFault(FaultCode)` requires a `FaultPolicy`** to know whether the fault is recoverable in this build's policy. F-D5 owns that.

The right boundary: `HarnessChannel` reads/writes the F-A3 `#[repr(C)]` blocks; F-D2's control plane drives the dispatch. F-A7's contract is "tell me when a `HarnessCommandBlock` arrives at the doorbell address; I'll surface it." F-D2's contract is "I see the surfaced `HarnessCommand`; I dispatch and write the result."

The shape:

```rust
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct HarnessSlot {
    pub sram_bank: u8,             // SRAM bank in which the blocks live (MBC5: 0..=15)
    pub command_addr: u16,         // SRAM-resident HarnessCommandBlock base
    pub result_addr: u16,           // SRAM-resident HarnessResultBlock base
    pub doorbell_addr: u16,         // single-byte ready-edge signal
}

pub struct HarnessChannel {
    slot: HarnessSlot,
    /// Last sequence number this channel observed in a successful `read_command`.
    /// The doorbell is only an edge signal; the u32 sequence comes from the command block.
    last_seen_seq: u32,
}

/// F-A3 wire-format wrapper. F-A7 does not re-mirror the field layout locally; it carries
/// the full F-A3 block and exposes convenience accessors (`block.seq()`, `block.op()`, `block.payload()`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessCommand {
    pub block: gbf_abi::harness::HarnessCommandBlock,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessResult {
    pub block: gbf_abi::harness::HarnessResultBlock,
}

impl HarnessChannel {
    pub fn new(slot: HarnessSlot) -> Self;
    pub fn slot(&self) -> HarnessSlot;
    pub fn last_seen_seq(&self) -> u32;
    /// Internal: parse the command block from a memory accessor that the parent
    /// `Emulator` provides. Switches to `slot.sram_bank` first; uses adapter-internal
    /// peek so guest memory traps and trace events are not triggered.
    pub(crate) fn read_command_from<M: HarnessMemory>(&mut self, mem: &M) -> Result<Option<HarnessCommand>, EmuError>;
    pub(crate) fn write_result_to<M: HarnessMemory>(&mut self, mem: &mut M, result: HarnessResult) -> Result<(), EmuError>;
}

/// Trait implemented by the parent `Emulator` for adapter-internal access. Public
/// consumers go through `Emulator::poll_harness` / `Emulator::write_harness_result`,
/// not this trait.
///
/// Per §3.7.1 finding 9: M0 accesses cartridge SRAM **directly by `(bank, addr)`** through
/// the cartridge.ram backing store rather than mutating the emulated MBC5 SRAM bank
/// register and going through `GameBoy::write`. This avoids two problems: (a) generic
/// side-effect-free bank switching is not in the public gameroy-core API; (b) a
/// harness-driven MBC bank-register write would itself emit a `RomBankSwitch` /
/// `SramBankSwitch` trace event and pollute the guest trace.
pub(crate) trait HarnessMemory {
    /// Read `addr` (in the 0xA000..=0xBFFF SRAM window) from the cartridge RAM bank
    /// `bank`, computed as `cartridge.ram[bank * 0x2000 + (addr - 0xA000)]`. Bounds-checked
    /// against the cartridge's actual SRAM size; returns `EmuError::HarnessSramOutOfRange`
    /// when the offset exceeds `cartridge.ram.len()`.
    fn read_sram_bank(&self, bank: u8, addr: u16) -> Result<u8, EmuError>;
    fn read_sram_bank_range(&self, bank: u8, addr: u16, len: usize) -> Result<Vec<u8>, EmuError>;
    fn write_sram_bank(&mut self, bank: u8, addr: u16, value: u8) -> Result<(), EmuError>;
}
```

For non-MBC5 cartridges (a future concern), F-A7 grows a `BankedSramDebugAccess` trait that the cartridge type implements; the spike confirms whether `gameroy::gameboy::cartridge::Cartridge::ram` is publicly accessible at the pinned rev. If cartridge RAM is private, the harness verbs return `EmuError::HarnessSramAccessUnavailable` and the spike-pinning bead opens an upstream ask. **No path through `GameBoy::write` for harness writes is acceptable**, because that would route through MBC control logic, advance the clock, and emit guest trace events.

The public verbs live on `Emulator`, not on `HarnessChannel`, to avoid the borrow-checker problem of "the channel needs `&Emulator` to read memory but `Emulator::harness()` mutably borrows the channel out of `Emulator`." From a consumer's perspective:

```rust
emu.attach_harness(HarnessSlot { sram_bank: 0, command_addr: 0xA000, result_addr: 0xA040, doorbell_addr: 0xA080 });
match emu.poll_harness()? {
    Some(cmd) => { /* dispatch via F-D2 */ emu.write_harness_result(result)?; }
    None => { /* doorbell hasn't fired */ }
}
```

`Emulator::poll_harness` returns:

- `Ok(None)` when the doorbell byte has not been written since `last_seen_seq` (no new command).
- `Ok(Some(cmd))` when the doorbell signals readiness *and* the in-block magic + sequence parse correctly.
- `Err(EmuError::HarnessMagicMismatch)` when the magic header at `slot.command_addr` is wrong (e.g., `"HCMD"` per F-A3).
- `Err(EmuError::HarnessSequenceMismatch)` when the **command block's** `seq` is not the expected next sequence for this channel. (The doorbell byte is an *edge* signal, not the source of the `u32`; the `u32` is in the command block.)

`Emulator::write_harness_result` writes the F-A3 `HarnessResultBlock` bytes to `slot.result_addr` and then the doorbell byte by indexing `cartridge.ram` directly at `(slot.sram_bank, addr)` — never via `GameBoy::write` and never by mutating the emulated MBC5 SRAM bank register (per §3.7.1 finding 9). All four operations go through `HarnessMemory::write_sram_bank`; **they do not emit guest `MemoryWrite` trace events, do not trigger guest memory traps, and do not advance the clock**. They optionally surface as `TraceOrigin::HostPoke` when explicit debug auditing is enabled.

The `op_discriminator` → `HarnessOp` mapping is F-D2's: F-D2 owns the dispatch table. F-A7 only carries the F-A3 block. This decoupling is the single most important design decision in this module.

### 8.2 `HarnessSlot::address()` choice

F-A7 does not pin specific addresses for the harness command/result blocks. The `HarnessSlot` is constructor-provided. F-D2 (or its tests) chooses `command_addr`, `result_addr`, `doorbell_addr` against the build's SRAM map; F-A7 just consumes them. This keeps F-A7 ignorant of build-specific layout — a property that prevents F-A7 from ever needing to know about `SramPagePlan`.

### 8.3 Tests

```bash
cargo test -p gbf-emu -- harness::poll_returns_none_when_no_doorbell
cargo test -p gbf-emu -- harness::poll_parses_f_a3_command_block
cargo test -p gbf-emu -- harness::poll_rejects_wrong_magic
cargo test -p gbf-emu -- harness::poll_rejects_seq_mismatch_on_command_block_seq  # not on doorbell value
cargo test -p gbf-emu -- harness::write_result_writes_result_block_bytes
cargo test -p gbf-emu -- harness::read_command_does_not_read_result_block
cargo test -p gbf-emu -- harness::poll_does_not_emit_guest_memory_writes
cargo test -p gbf-emu -- harness::poll_does_not_trigger_guest_memory_traps
cargo test -p gbf-emu -- harness::write_result_does_not_emit_guest_memory_writes
cargo test -p gbf-emu -- harness::write_result_does_not_advance_clock        # SRAM-direct path bypasses GameBoy::write
cargo test -p gbf-emu -- harness::write_result_does_not_emit_sram_bank_switch_event  # MBC5 bank register is untouched
cargo test -p gbf-emu -- harness::sram_out_of_range_returns_typed_error
cargo test -p gbf-emu -- harness::slot_carries_sram_bank
cargo test -p gbf-emu -- harness::last_seen_seq_advances_only_on_successful_read
```

## 9. Cross-module conformance

A single integration test, `gbf-emu/tests/cross_module_conformance.rs`, walks every cross-module invariant:

- `Emulator::load_rom` → `rom_sha256()` matches a Hash256 of the input bytes.
- `Snapshot::lineage.rom_sha256` always equals `Emulator::rom_sha256()` for the snapshot's owner.
- `Snapshot::lineage.policy_fingerprint` always equals `policy().fingerprint()`.
- `restore` rejects every lineage-mismatch case with the corresponding `EmuError` variant.
- `TrapDispatcher::add_*` returns a `BreakpointId` that is later present in `list()`; `remove` drops it.
- `TraceCursor::drain` produces events with `region == gbf_hw::memory::classify(addr)` for every `MemoryWrite`.
- `Emulator::poll_harness` correctly parses the F-A3-defined `HarnessCommandBlock` layout (round-trip: pre-populate the SRAM-resident block via adapter-internal pokes against `slot.command_addr`, then `poll_harness` returns the parsed value).

All of these run as part of `cargo test -p gbf-emu` in pre-commit.

## 10. Dependency surface

`gbf-emu/Cargo.toml` after F-A7 closure:

```toml
[package]
name = "gbf-emu"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
publish = false
build = "build.rs"   # injects the gameroy-core git rev as a const

[dependencies]
gbf-abi        = { path = "../gbf-abi" }
gbf-foundation = { path = "../gbf-foundation" }
gbf-hw         = { path = "../gbf-hw" }
# Headless emulator core. The repo's root `gameroy` package is the GUI/application crate
# with UI/audio default features and library-target `gameroy_lib`; we depend on the
# headless `gameroy-core` package whose library target is `gameroy`. The rename keeps
# imports as `use gameroy::...`.
gameroy        = { package = "gameroy-core", git = "https://github.com/Rodrigodd/gameroy", rev = "<pinned-sha>", default-features = false, features = ["io_trace"] }
serde          = { workspace = true, default-features = false, features = ["derive", "alloc"] }
thiserror      = { workspace = true }
sha2           = { workspace = true }   # only if gbf-foundation's Hash256 does not already expose a hashing constructor

[dev-dependencies]
serde_json     = { workspace = true }
```

Notes:

- The dependency is on **`gameroy-core`** (the headless package), renamed locally to `gameroy` so source imports do not drift. The root `gameroy` package is the GUI/application crate and is **not** a F-A7 dependency.
- The pin is by **git revision** (not SemVer, not branch). The exact rev is decided at F-A7 closure and recorded in `Cargo.lock`. The rev is also embedded as a `const GAMEROY_GIT_REV: GitSha` by `build.rs` (reading cargo metadata) for use in `EmuVersionTag`. Bumping the rev is a follow-up bead.
- Additional `gameroy-core` features beyond `io_trace` are enabled only if the §3.7 adapter spike proves they are required.
- `thiserror` is added because F-A7 derives `thiserror::Error` on `EmuError` and its sub-errors.
- `sha2` is added only if `gbf-foundation`'s `Hash256` does not already expose a `Hash256::sha256(bytes: &[u8])` constructor. If it does, this dependency is dropped.
- `serde_json` moves from `[dependencies]` (where the existing scaffold has it) to `[dev-dependencies]`. Production code doesn't need JSON; tests do.
- No dependency on `gbf-bench`, `gbf-codegen`, `gbf-runtime`, `gbf-asm`, or any training-side crate. Adding any of these would create a cycle or destroy the dependency direction.

## 11. Implementation notes

### 11.1 `no_std` posture

`gbf-emu` is **not** `no_std` and is **not** intended to become `no_std`. `gameroy-core` is `std`-using; `gbf-emu` runs on the host. Engineering rule 6 ("`no_std + alloc` capable where practical") is explicitly opted out here because the dependency makes it impractical. The crate-root attribute is `#![forbid(unsafe_code)]`, not `#![no_std]`.

If `gameroy-core` ever ships a `no_std` mode, a follow-up bead can flip `gbf-emu` to `#![no_std]` mechanically (the modules use no `std::collections::HashMap` and no `std::sync::*`; they use `Vec`, `String`, and `VecDeque`, all of which are in `alloc`). That's a future concern, not a F-A7 deliverable.

### 11.2 The `gameroy-core` git-rev pin

The pin is recorded in three places:

- `gbf-emu/Cargo.toml` → `[dependencies] gameroy = { package = "gameroy-core", git = "...", rev = "..." }`
- `Cargo.lock` (at the workspace root) → records the resolved `gameroy-core` source git rev
- `gbf-emu/build.rs` reads cargo metadata and emits a `const GAMEROY_GIT_REV: GitSha` consumed by `EmuVersionTag`

The pin is surfaced in the review packet and in the F-A7 closure comment. Bumping the pin requires a new bead because:

1. Every existing `Snapshot` in any saved session file becomes invalid (`SnapshotEmuVersionMismatch` keyed on `gameroy_git_rev`).
2. The determinism contract (`tests/determinism_single_path`) must be re-asserted.
3. Any subtle behavior change in `gameroy-core` can ripple into trace events, cycle counts, and memory traces — all of which downstream consumers depend on.

The pin is a load-bearing artifact of F-A7; do not change it casually.

### 11.3 Errors

F-A7's error inventory is **canonical and consistent across §0, §1, §11**. Budget exhaustion is *not* an error; it is a `RunOutcome` variant.

```rust
#[derive(Debug, thiserror::Error)]
pub enum EmuError {
    #[error("ROM load failed: {0}")]
    RomLoad(String),
    #[error("Step failed: {0}")]
    Step(String),
    #[error("Trap predicate failed: {0}")]
    TrapPredicate(#[from] TrapPredicateError),
    #[error("Snapshot save failed: {0}")]
    SnapshotSave(String),
    #[error("Snapshot load failed: {0}")]
    SnapshotLoad(String),
    #[error("Snapshot ROM mismatch: snapshot is for {expected:?}, current ROM is {observed:?}")]
    SnapshotRomMismatch { expected: Hash256, observed: Hash256 },
    #[error("Snapshot policy mismatch: expected fingerprint {expected:?}, current is {observed:?}")]
    SnapshotPolicyMismatch { expected: Hash256, observed: Hash256 },
    #[error("Snapshot emu-version mismatch: snapshot was {expected:?}, current is {observed:?}")]
    SnapshotEmuVersionMismatch { expected: EmuVersionTag, observed: EmuVersionTag },
    #[error("Harness magic mismatch: observed {observed:?}, expected {expected:?}")]
    HarnessMagicMismatch { observed: [u8; 4], expected: [u8; 4] },
    #[error("Harness sequence mismatch: observed {observed}, expected {expected}")]
    HarnessSequenceMismatch { observed: u32, expected: u32 },
    #[error("Trace capacity exceeded: drop policy is HaltAndError")]
    TraceCapacityExceeded,
    #[error("Memory access error: {0}")]
    MemoryAccess(String),
    #[error("Determinism error: {0}")]
    Determinism(String),
}

#[derive(Debug, thiserror::Error)]
pub enum TrapPredicateError {
    #[error("source predicate requires F-A8's evaluator; F-A7 cannot interpret JS")]
    SourceRequiresEvaluator,
    #[error("predicate closure returned an error: {0}")]
    Closure(String),
}

#[derive(Debug, thiserror::Error)]
pub enum AddressRangeError {
    #[error("AddressRange::new: start {start:#06x} > end_inclusive {end_inclusive:#06x}")]
    StartAfterEnd { start: u16, end_inclusive: u16 },
}

#[derive(Debug, thiserror::Error)]
pub enum TrapPersistenceError {
    #[error("trap {id:?} is closure-only and has no recorded source form")]
    ClosureOnly { id: BreakpointId },
}
```

Every variant carries enough state to reproduce the failing input. This matches F-A1 / F-A2 / F-A3's error style and `CONSTITUTION.md` §V.3.

**Pre-1.0 source-break policy.** Adding a new public exhaustive enum variant (to `EmuError`, `RunOutcome`, `StepOutcome`, `NormalizedTraceEvent`, `TrapKind`, etc.) is **not** a backward-compatible change for downstream code that matches exhaustively. F-A7 deliberately accepts this pre-1.0 because compile errors are preferred over silent fallthrough — the costs go to consumers as `cargo check` failures, not to production as undefined behavior. The RFC therefore does not call variant additions "additive" or "preserve backward compat"; it calls them "intentional source breaks until `gbf-emu` reaches 1.0." If a consumer crate explicitly wants forward-compatibility, F-A7 may add `#[non_exhaustive]` to the relevant enum in a follow-up bead.

### 11.4 Performance targets

`gbf-emu` is consumed at test, bench, and debug time. It is *not* a hot path in production (production runs on actual Game Boy hardware). Targets:

- `Emulator::step` — bounded by gameroy's per-instruction cost; F-A7 adds at most one `match` over the trap registry. Sub-microsecond.
- `run_for(budget)` — bounded by gameroy-core's runtime; F-A7 adds at most one trap dispatch per step.
- `Snapshot::serialize` / `deserialize` — bounded by gameroy's save state size (typically <16 KiB) plus a `Hash256` triple. Sub-millisecond.
- `TraceCursor::record` — `O(1)` amortized; the underlying `VecDeque` push/pop is `O(1)`.

The crate adds ~1,200 lines to the workspace; total cargo build cost is dominated by gameroy's compile time.

### 11.5 Versioning

`gbf-emu` is `0.1.0` for M0. The crate stabilizes at `1.0.0` only after F-A8 (`gbf-debug`) and F-D2 (Harness Control Plane) have validated the API against real consumers. Pre-`1.0.0`, breaking changes to the `Emulator` verb set or `EmuError` variants are coordinated through the bead graph.

### 11.6 Documentation

Every public item carries a doc comment with at minimum:

- A short description.
- A planv0.md / RFC-section reference where applicable.
- A `# Examples` block if the item has non-trivial usage.
- A `# Provenance` block citing the planv0.md line that motivated the item.

Doc comments are linted via `rustdoc`'s `--deny broken-intra-doc-links` and `--deny missing-docs`.

## 12. Implementation order

Within F-A7, the open tasks have a real DAG:

```
T-A7.1 scaffold   ────┐
                       ├──► T-A7.2 adapter+primitives ────┐
                       │                                    ├──► T-A7.4 trap dispatcher ──┐
                       │                                    │                              ├──► T-A7.5 trace_ring
T-A7.3 determinism  ───┘                                    │                              └──► T-A7.6 harness
                                                             │
                                                  (T-A7.4 also blocks T-A7.5/T-A7.6)
```

**Recommended order:**

1. **T-A7.1 scaffold + Cargo.toml + lib.rs** (foundational). Half a day. Adds gameroy dependency, renames the four stub files to the planv0.md-line-207 layout, adds `#![forbid(unsafe_code)]`, declares the six modules with `//! Module stub.` placeholders.
2. **T-A7.2 adapter.rs + primitives.rs** (parallel with T-A7.3). Two days. The largest module by LOC; wraps gameroy and exposes the seven verbs.
3. **T-A7.3 determinism.rs** (parallel with T-A7.2). Half a day. Smallest module; the policy + builder + fingerprint.
4. **T-A7.4 trap.rs**. One day. Registry + per-cycle dispatch + closure/source predicate enum.
5. **T-A7.5 trace_ring.rs** (parallel with T-A7.6). One day. Normalized event format + cursor + bank-switch detection.
6. **T-A7.6 harness.rs** (parallel with T-A7.5). Half a day. Read/write the F-A3 blocks; doorbell tracking.

**Total: ~5.5 days of focused work.** The critical path is scaffold → adapter+primitives → trap → trace+harness. Some work parallelizes.

**PR shape (closure):** F-A7 ships in a single PR that lands all six modules together. The earlier bead-comment proposed a step-by-step landing; in practice the modules are too tightly cross-linked (trap dispatcher needs `Emulator`; trace cursor needs `MemoryRegion` and the spike-pinned access-observation mechanism; harness needs `peek_range` / `poke`) for the split to reduce review effort. A single PR closes T-A7.1 through T-A7.6 and the parent feature bead `bd-3mxe` in one step.

## 13. Testing strategy summary

| Layer                    | Coverage                                                                                                  |
|--------------------------|-----------------------------------------------------------------------------------------------------------|
| Compile-only spike       | `tests/gameroy_api_spike.rs` (or `examples/gameroy_api_spike.rs`) compiles against the pinned `gameroy-core` rev and proves the construction / step / save-state / access-observation / framebuffer / joypad paths the RFC names. The implementation PR amends the RFC if any assumption fails. |
| Type-level (compile-time)| `EmuError`, `TrapKind`, `TrapAction`, `CartridgeRtcMode`, `SaveStateMetadataMode`, `PowerOnRamPolicy`, `AudioOutputMode`, `NormalizedTraceEvent`, `TraceOrigin`, `BankSwitchSource`, `RunOutcome`, `StepOutcome`, `CpuIdleState`, `MemoryAccessKind`, `TraceDropPolicy`, `BootMode`, `CycleBudget` are exhaustive enums. |
| Unit / property          | Per-module tests against gameroy-core behavior (register state, bus vs. peek, snapshot round-trip). Trace normalization fixture-driven. |
| Integration              | `tests/determinism_single_path.rs` — same ROM + same policy → byte-equal `(framebuffer, regs, mem_dump)` after fixed `ClockCycles`. `tests/tiny_rom_boots.rs` — F-A1's deferred boot test (PC reaches `$0150` within `MAX_TINY_ROM_BOOT_BUDGET` from `BootMode::PostBootDmg`). `tests/snapshot_round_trip.rs` — save → restore preserves state byte-for-byte; lineage rejection is typed. `tests/cross_module_conformance.rs` — every cross-module invariant. |
| Snapshot                 | `Snapshot::lineage` JSON round-trip. `NormalizedTraceEvent` JSON round-trip per variant. `TrapSpec` / `PredicateSpec` JSON round-trip. Save-state header bytes are byte-equal across runs. |
| Negative                 | `EmuError::SnapshotRomMismatch` / `SnapshotPolicyMismatch` / `SnapshotEmuVersionMismatch` on mismatched lineage. `EmuError::HarnessMagicMismatch` on wrong magic. `EmuError::HarnessSequenceMismatch` on command-block seq drift. `EmuError::TraceCapacityExceeded` on overflow with `HaltAndError`. `TrapPredicateError::SourceRequiresEvaluator` on raw `Source` predicate. `AddressRangeError::StartAfterEnd` on inverted ranges. `TrapPersistenceError::ClosureOnly` on attempted serialization of closure-only entries. |
| Skill checklist          | Constructor-validated newtypes (`AddressRange::new`, `Flags::new` masks low nibble). Effect-classifier-style traversals (`MemoryRegion` classification of every event). JSON-facing schemas have round-trip tests. |
| Documentation-only smoke | `single_substrate_smoke::grep_no_other_gameroy_imports` ships as `#[ignore]` smoke test in F-A7; **does not enforce anything** while ignored; promotion to a workspace gate lands once `gbf-test` exists. |

All tests run as part of the workspace pre-commit hook (`cargo test --workspace --all-features`). The `tests/tiny_rom_boots.rs` test is *the* load-bearing acceptance gate that F-A7 actually runs ROMs end-to-end.

## 14. Resolved questions

These were the questions I planned to surface in PR review. Each is now resolved; the decisions below are load-bearing for closure.

1. **The dependency is on `gameroy-core`, not the root `gameroy` package.** The repo's root package is the GUI/application crate (library target `gameroy_lib`, with UI/audio default features); the headless emulator core is the `gameroy-core` package whose library target is named `gameroy`. F-A7's `Cargo.toml` uses `package = "gameroy-core"` to rename it locally so source imports remain `use gameroy::...`.
2. **The exact gameroy-core API surface is pinned by a compile spike, not by guessing.** The §3.7 spike compiles against the pinned rev and proves construction / step / save-state / access-observation / framebuffer / joypad paths. No `Console` / `Game` symbol is named in this RFC unless the spike proves it.
3. **Cycle units are typed.** `ClockCycles(u64)` and `MCycles(u64)` are separate newtypes. `CycleBudget` is the input to `run_for` and `run_until_pc`. The clock-count source of truth is gameroy-core's clock; M-cycle views are derived as `clock / 4`.
4. **Budget exhaustion is a `RunOutcome`, not an `EmuError`.** `RunOutcome::BudgetElapsed { observed, requested }` is the normal outcome of `run_for(budget)`. An earlier draft put `RunBudgetExhausted` in `EmuError` and that contradicted the `RunOutcome` shape; the unified shape lives in `RunOutcome`.
5. **`Emulator` is `&mut self`-method-style, not callback-style.** Callback-style would force the consumer to maintain its own state machine across callbacks; the verb form keeps state in the `Emulator` and the consumer's loop reads naturally. The cost is "you cannot interrupt a `run_for` from another thread"; F-A7 commits to that limitation explicitly (single-threaded substrate by design).
6. **Bus access (side-effecting) is split from debugger peek/poke (side-effect-free).** Gameroy-core's `read(&self)` can update lazy components (e.g., timer state on IO read) through interior mutability. A trap predicate that calls `bus_read` could perturb timing; one that calls `peek` cannot. The split is mandatory.
7. **`Predicate::Closure` is `FnMut(&TrapContext<'_>) -> Result<bool, _>` and is not `Send + Sync`.** The closure form needs to mutate captured counters (`FnMut`), report errors typed (`Result`), and live in a single-threaded runtime (no `Send + Sync`).
8. **`Predicate` is not `Serialize`; persistence goes through `TrapSpec` / `PredicateSpec`.** Predicates carry closures that cannot be serialized; `TrapSpec` is the explicit persistence shape consumed by F-A8 sessions. `TrapDispatcher::export_persistable_specs()` returns `Vec<TrapSpec>` and refuses to emit closure-only entries.
9. **PC traps fire BEFORE executing the instruction at the registered address.** This is what every debugger expects, including gameroy-core's own debugger as documented upstream. `run_until_pc(pc, budget)` returns immediately if the emulator is already positioned at `pc`.
10. **Trap dispatch is wired to whichever access-observation mechanism the §3.7 spike proves.** F-A7 does not claim a public per-memory-access callback unless the pinned rev exposes one. Likely candidates: the `io_trace` feature buffer, debugger internals, interpreter instrumentation, or an F-A7-local wrapper.
11. **`NormalizedTraceEvent::TrapHit` is F-A7-local, not an F-A3 `TraceEvent::TrapHit`.** F-A7 does not invent F-A3 wire-format types. Continue-action trap logging uses the local variant.
12. **`MemoryWrite` carries `bank: BankSnapshot` and `origin: TraceOrigin`.** Bank context is required to compare traces across bank switches; origin distinguishes guest writes from host pokes (so harness pokes do not pollute the guest trace).
13. **The trace cursor does not deduplicate identical guest events.** Two identical writes to the same address in consecutive cycles are real. Any duplicate suppression must be backed by a fixture proving the duplicate is instrumentation noise.
14. **`HarnessChannel::poll` lives on `Emulator`, not on the channel.** Putting it on the channel forces the consumer through `Emulator::harness().unwrap().read_command(&emu)`, which fights the borrow checker because `harness()` mutably borrows out of `emu`. The verbs live on `Emulator`.
15. **`HarnessSlot` carries `sram_bank`.** SRAM is banked under MBC5; a raw `u16` address is ambiguous without the bank. The control plane switches to `slot.sram_bank` before every harness operation.
16. **The harness doorbell is an edge signal; the `u32` sequence is in the command block.** A previous draft compared the doorbell byte to the sequence; that confused the role of the doorbell.
17. **`HarnessCommand` / `HarnessResult` wrap F-A3's `#[repr(C)]` blocks directly.** No local mirror of the field layout in `gbf-emu`. This honors the "no bytewise re-implementation" rule.
18. **Snapshot identity uses `gameroy_git_rev`, not `gameroy_semver`.** A published version can stay constant while source moves. Save-state bytes are tied to source.
19. **Save-state metadata timestamp is fixed deterministically.** gameroy-core's save-state header carries a timestamp; if F-A7 ever passes wall-clock there, snapshot bytes drift even when emulation is fully deterministic. `FIXED_SAVE_STATE_UNIX_MS` is separate from `FIXED_CARTRIDGE_RTC_UNIX_MS`.
20. **`Snapshot` captures gameroy-core state only, not adapter state.** Traps, trace cursor, harness sequence, and F-A8 session metadata are *not* part of `Snapshot`. F-A8 wraps `Snapshot` with the rest of the session for the on-disk format.
21. **The adapter does not maintain its own `cycle_count`.** A parallel counter desynchronizes from gameroy-core's clock across save/restore, post-boot init, and any debugger read that updates lazy components. The source of truth is the core; `clock_count()` and `m_cycle_count_floor()` are derived views.
22. **RTC pinning is a no-op for MBC5.** MBC5 has no RTC surface; Pan Docs §"MBC3" describes the RTC. The `cartridge_rtc` field exists for future MBC3/HuC-3 cartridges; for M0 it is unobserved.
23. **`PowerOnRamPolicy` replaces `rng_seed: u64`.** The Game Boy has no inherent RNG; the determinism question is uninitialized RAM and any host entropy the core exposes. The policy variants are typed; if the spike proves the core is fully deterministic without a seed, the non-default variants may be removed in a follow-up.
24. **`AudioOutputMode::Disabled` means no host audio sink, not "APU off".** The APU is part of CPU-visible state evolution; disabling it could change behavior. Host output is the part headless tests do not need.
25. **`Flags(u8)` is a constructor-validated newtype.** The low nibble is masked off in `Flags::new`. `set_regs` returns `Result` to surface gameroy-core rejections.
26. **`StepOutcome::Idle` and `RunOutcome::Idle` carry a `CpuIdleState`.** HALT and STOP have different semantics; HALT is not terminal (an interrupt can wake it). The earlier `Halted` name was wrong.
27. **`AddressRange::end_inclusive`, not `end_exclusive`.** Memory-access traps describe ranges naturally as inclusive (e.g., "the IO region is `$FF00..=$FF7F`"). Layout pass uses exclusive forms (F-A2 §0.0.5); trap dispatchers use inclusive forms. The struct is constructor-validated.
28. **F-A1's `tiny_rom_boots` test lives in `tests/tiny_rom_boots.rs`, not in the example.** The example demonstrates *usage*; the test asserts *correctness*. The fixture is checked into `gbf-emu/tests/fixtures/tiny_rom.gb` with provenance metadata; `cargo test` does not shell out to `cargo run -p gbf-asm`.
29. **`#![forbid(unsafe_code)]` is the primary unsafe gate.** Compiler-enforced, not grep-based.
30. **No `bytemuck` dependency.** F-A3's `from_le_bytes` / `to_le_bytes` helpers are sufficient; F-A7 calls them directly. Adding `bytemuck` would add an unsafe-using dependency for marginal ergonomic gain.
31. **Pre-1.0, new public exhaustive enum variants are intentional source breaks.** F-A7 prefers compile errors over silent fallthrough; consumers `match` exhaustively and pay the upgrade cost. The RFC does not call variant additions backward-compatible.
32. **Memory traps under the `io_trace` backend are post-instruction traps, not exact pre-access traps.** PC traps remain pre-instruction. Memory traps fire at the instruction boundary after the matching access; the reported emulator state is post-instruction. Exact pre-access memory traps are an explicit non-goal for M0 — they require an upstream-instrumented `gb_read` / `gb_write` callback in `gameroy-core`, which is deferred to a follow-up bead. (See §3.7.1 finding 4.)
33. **`BootMode::BootRom` carries the actual 256 boot-ROM bytes, not just a hash.** `gameroy::gameboy::GameBoy::new` takes `Option<[u8; 0x100]>`; a hash alone cannot construct the emulator. Shape: `BootRom(BootRomImage { bytes: Box<[u8; 0x100]>, sha256: Hash256 })`. The hash is for lineage validation. (See §3.7.1 finding 1.)
34. **`Regs.ime` is tri-state `ImeSnapshot { Disabled, Enabled, ToBeEnable }`, not `bool`.** Gameroy's `ImeState` is tri-state and the `ToBeEnable` state is observable around `EI` + interrupts. A `bool` would silently lose it. HALT/STOP execution state is *not* part of `Regs`; it lives only in `StepOutcome::Idle` / `RunOutcome::Idle`. (See §3.7.1 finding 2.)
35. **`peek` / `poke` are side-effect-free *only* for raw-backed regions** (ROM, WRAM, HRAM, SRAM at an explicit bank, and — if the spike confirms — VRAM/OAM). For unsupported regions (notably IO `$FF00..=$FF7F`), they return `EmuError::DebugMemoryUnsupported { addr }`. IO inspection goes through `bus_read`. (See §3.7.1 finding 3, Option B.)
36. **`io_trace` is drained per-`interpret_op`, never accumulated across instructions.** The `io_trace` tuple's first byte packs `kind | ((clock_count & !3) >> 1)`, so absolute `ClockCycles` for each event are reconstructed in the adapter from the step's `start_clock` / `end_clock` plus per-access ordering. (See §3.7.1 finding 5.)
37. **`TraceOrigin::HostBus` distinguishes side-effecting `bus_read`/`bus_write` from raw-debug `poke`.** Variants: `GuestCpu | HostBus | HostPoke | Dma`. `HostBus` advances the clock; `HostPoke` does not. (See §3.7.1 finding 6.)
38. **`AudioOutputMode` is single-variant (`Disabled`) for M0.** `gbf-emu` depends on the headless `gameroy-core`, which has no host audio sink. The builder's `with_audio_output_enabled()` was removed; re-enabling host audio would require depending on the GUI/application package, which §10 forbids. APU emulation still evolves normally. (See §3.7.1 finding 7.)
39. **`PowerOnRamPolicy::Seeded` was removed for M0.** The Game Boy has no built-in RNG and `gameroy-core` exposes no seedable power-on-RAM API. `FixedFill` becomes `FixedFill { wram, hram, cartridge_ram }` (per-region) because gameroy-core's defaults differ per region (`0xFF` for WRAM/HRAM, `0x00` for cartridge RAM). Reintroduce `Seeded` only via a follow-up bead if an upstream hook lands. (See §3.7.1 finding 7.)
40. **`CartridgeRtcMode::Fixed` is in the policy fingerprint but only *applied* when gameroy-core exposes a cartridge RTC hook.** For M0/MBC5 it is unobserved. Loading an RTC-bearing cartridge (MBC3 with timer, HuC-3) under F-A7 returns `EmuError::Determinism { reason }` unless RTC control is available. (See §3.7.1 finding 7.)
41. **`SnapshotLineage` carries `boot: BootModeLineage`.** `gameroy-core`'s save state does not include the boot-ROM bytes themselves, so a snapshot taken under one boot ROM is under-specified if restored under a different one. F-A7 carries the boot fingerprint; `restore` fails with `EmuError::SnapshotBootMismatch` on mismatch. (See §3.7.1 finding 8.)
42. **The harness is MBC5-SRAM-direct for M0.** `HarnessChannel` reads/writes `cartridge.ram` directly by `(sram_bank, addr)` (offset = `sram_bank * 0x2000 + (addr - 0xA000)`). It does **not** call `GameBoy::write` and does **not** mutate the emulated MBC5 SRAM bank register. This avoids both clock advancement and harness-driven `RomBankSwitch` / `SramBankSwitch` trace pollution. The `HarnessMemory` trait exposes `read_sram_bank` / `write_sram_bank` / `read_sram_bank_range` instead of `switch_sram_bank` + `peek` / `poke`. (See §3.7.1 finding 9.)
43. **`bus_read` / `bus_write` are "adapter-synthesized CPU-bus operations".** They route through gameroy's public `read` / `write`, advance the clock by one CPU memory-access quantum, and emit trace events with `TraceOrigin::HostBus`. F-A7 does *not* claim bit-exact equivalence to the interpreter's private `gb_read` / `gb_write`. (See §3.7.1 finding 10.)
44. **`MemoryAccessKind` is `InstrFetch | DataRead | Write`.** Under the `io_trace` backend, opcode/operand fetches and data reads are folded together at the wire level; `MemRead` range traps fire on instruction fetches inside the range. Tests that need to filter fetches must do so in the predicate. (See §3.7.1 finding 4.)

## 15. Risks

| Risk                                                                         | Likelihood | Mitigation                                                                                                                                  |
|------------------------------------------------------------------------------|------------|---------------------------------------------------------------------------------------------------------------------------------------------|
| gameroy-core's actual public API does not match this RFC's assumptions       | Medium     | The §3.7 adapter spike is **a precondition for implementation approval**. If the spike contradicts an assumption, the implementation PR amends the RFC rather than fight the API. |
| Pinned `gameroy-core` rev exposes no per-memory-access callback              | Medium     | The §3.7 spike documents the actual access-observation surface; if no public callback exists, F-A7 falls back to the `io_trace` buffer or an interpreter wrapper. The RFC's §6 / §7 wording is rewritten in the implementation PR if needed. |
| `gameroy-core`'s library API drifts across pin bumps                         | Medium     | Pinned git rev; bumping is a follow-up bead; review packet records the exact `gameroy-core` git rev; integration tests pin the expected behavior. |
| Determinism contract regresses (host entropy leaks; RTC drifts; save-state metadata drifts) | Low | `tests/determinism_single_path.rs` runs in pre-commit; `(framebuffer, regs, mem_dump)` asserted byte-equal across two runs; `save_state_header_timestamp_fixed` asserts snapshot byte-equality. |
| Trap dispatcher's per-step cost dominates `run_*` performance                | Low        | M0 expects single-digit traps; the linear scan is fine; if it ever matters, an interval tree is a follow-up.                                |
| `NormalizedTraceEvent` shape turns out wrong for F-D3                        | Medium     | Schema is informed by planv0 line 313 + F-D3's bead description. **New public variants are intentional source breaks pre-1.0**, not additive — consumers `match` exhaustively and the compile error surfaces the change. |
| F-A2 / F-A3 land late and F-A7 has to mock their types                       | Low        | F-A7 explicitly blocks on F-A2 / F-A3 (per the bead graph); local re-declaration is forbidden by §2.1 and the single-source-of-truth rule. |
| gameroy-core's save state format is unstable across revs                     | High       | `Snapshot::lineage.emu_version.gameroy_git_rev` rejects mismatched restores with `SnapshotEmuVersionMismatch`. Pin bump invalidates all existing snapshots intentionally. The save-state metadata timestamp is fixed at `FIXED_SAVE_STATE_UNIX_MS`. |
| `Source` predicate gets evaluated by F-A7 by accident                        | Low        | `TrapPredicateError::SourceRequiresEvaluator` is a typed failure mode; F-A8's wrapper compiles `Source` into `Closure` before handing the dispatcher to F-A7's runtime. |
| F-A1's `tiny_rom.gb` byte content changes and breaks `tests/tiny_rom_boots`  | Very Low   | The fixture is checked into `gbf-emu/tests/fixtures/` with provenance metadata and a regeneration command. Any byte change is intentional and updates F-A7's test simultaneously. |
| F-D2 invents a different `HarnessSlot` layout and we have to rewrite         | Low        | `HarnessSlot` carries `sram_bank` explicitly so MBC5 banking is not silently assumed; `HarnessSlot` is a value object F-D2 constructs and F-A7 just consumes. |
| Adapter and gameroy-core's clock counters desynchronize                      | Low        | The adapter does not maintain its own counter; `clock_count()` reads gameroy-core directly. M-cycle views are derived. |
| Predicate borrow shape (`&Emulator` in closure) creates a borrow-checker fight | Resolved | Closures take `&TrapContext<'_>` with an `EmuReadOnlyView` instead of `&Emulator`. The dispatcher mutably borrows the registry; predicates have no mutable access to it. |
| Public method signature returns a private type                               | Resolved | `TrapDispatcher::remove` returns `bool`; richer info via `remove_entry() -> Option<RemovedTrap>` (public-only fields). |
| Adding `gameroy-core` dependency increases compile time noticeably           | Medium     | `gameroy-core` is the headless package without UI deps; compile time impact is bounded; CI caches the dependency. |

## 16. Claim-to-gate matrix (closure-style)

The closure skills (`.agents/skills/asm-bead-closure/SKILL.md`) require this for non-trivial beads. Pre-emptive matrix for F-A7 closure:

| Claim                                                                          | Gating test / artifact                                                                                  |
|--------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------|
| Dependency is `gameroy-core` (renamed to `gameroy`), not the root `gameroy` package | `Cargo.toml` review + `cargo tree -p gbf-emu` shows the `gameroy-core` package; review packet records  |
| The exact gameroy-core public API surface is pinned by a compile spike         | `tests/gameroy_api_spike.rs` (or the equivalent example) compiles against the pinned rev                |
| `Emulator::load_rom` produces an emulator whose `rom_sha256()` matches the input bytes | `adapter::load_rom_round_trip`                                                                |
| `step` advances the program counter by the encoded instruction length          | `adapter::step_advances_pc`                                                                             |
| `run_for(budget)` returns `RunOutcome::BudgetElapsed` when the budget runs out (not an error) | `adapter::run_for_budget_elapsed`                                                       |
| `run_until_pc` halts before the instruction at the requested PC and returns `TrapHit` | `adapter::run_until_pc_fires`                                                                    |
| `run_until_pc` returns immediately if already at the target PC                 | `adapter::run_until_pc_returns_immediately_when_already_there`                                          |
| `CycleBudget::Machine(M)` equals `Clock(M*4)`                                  | `adapter::cycle_budget_clock_machine_equivalence`                                                       |
| `regs()` and `set_regs()` round-trip the register state (within `Flags` mask)  | `adapter::regs_round_trip`, `adapter::set_regs_masks_low_flag_nibble`                                   |
| `bus_read` / `bus_write` are consistent within RAM                             | `adapter::bus_read_write_round_trip`                                                                    |
| `peek` is side-effect-free (does not advance clock or emit events)             | `adapter::peek_does_not_advance_clock`, `adapter::peek_does_not_emit_trace_events`                      |
| `poke` does not trigger guest memory traps                                     | `adapter::poke_does_not_trigger_guest_traps`                                                            |
| `framebuffer()` is byte-stable across reruns under the default policy         | `adapter::framebuffer_byte_stable`                                                                      |
| Joypad input is observed at `JOYP_REGISTER` after `set_joypad`                 | `adapter::joypad_round_trip`                                                                            |
| `Regs` round-trips through serde; `Flags` masks low nibble                     | `primitives::regs_serde_round_trip`, `primitives::flags_low_nibble_masked`                              |
| `Framebuffer::PartialEq` is byte-equality                                       | `primitives::framebuffer_eq_byte_stable`                                                                |
| `SnapshotLineage` round-trips through serde                                    | `primitives::snapshot_lineage_serde`                                                                    |
| `EmuVersionTag::gameroy_git_rev` matches the Cargo.lock pin                    | `primitives::emu_version_tag_records_git_rev`                                                           |
| `DeterminismPolicy::default()` is the locked-down policy (all four fields)    | `determinism::default_is_locked_down`                                                                   |
| `DeterminismPolicy::fingerprint()` is byte-stable for the default              | `determinism::fingerprint_stable`                                                                       |
| `fingerprint()` distinguishes any non-default policy                           | `determinism::fingerprint_distinguishes`                                                                |
| Builder produces a policy equivalent to a hand-constructed one                 | `determinism::builder_round_trip`                                                                       |
| Real-time cartridge RTC opt-out is via `with_real_time_cartridge_rtc()`       | `determinism::with_real_time_cartridge_rtc_named`                                                       |
| Audio-output opt-in is via `with_audio_output_enabled()`                       | `determinism::with_audio_output_enabled_named`                                                          |
| `PowerOnRamPolicy::Seeded` distinguishes via fingerprint                       | `determinism::power_on_ram_seeded_distinguishes`                                                        |
| Save-state header timestamp is fixed (snapshot bytes are byte-equal)           | `determinism::save_state_header_timestamp_fixed`                                                        |
| Same ROM + same policy → byte-equal trace + final state across runs            | `determinism::single_path` (the load-bearing one)                                                       |
| F-A1's `tiny_rom.gb` boots from `BootMode::PostBootDmg`: PC reaches `$0150`    | `tests/tiny_rom_boots.rs`                                                                                |
| Snapshot rejects mismatched ROM lineage                                        | `snapshot::rejects_rom_mismatch`                                                                        |
| Snapshot rejects mismatched policy fingerprint                                 | `snapshot::rejects_policy_mismatch`                                                                     |
| Snapshot rejects mismatched gameroy git rev                                    | `snapshot::rejects_emu_version_mismatch`                                                                |
| Snapshot does not capture installed traps / trace cursor / harness state       | `snapshot::captures_core_state_only`                                                                    |
| PC trap fires before the instruction at the registered address                 | `trap::pc_breakpoint_fires_before_instruction`                                                          |
| MemRead/MemWrite/MemRw traps fire on the correct access kind                   | `trap::mem_watchpoint_read`, `trap::mem_watchpoint_write`, `trap::mem_watchpoint_rw`                    |
| `Predicate::Always` fires every cycle the trap matches                         | `trap::predicate_always_fires`                                                                          |
| `Predicate::Closure` can read regs/pc/access through `TrapContext`             | `trap::predicate_closure_reads_trap_context`                                                            |
| `Predicate::Closure` cannot perturb emulator state via the read-only view      | `trap::predicate_closure_cannot_perturb_state`                                                          |
| `Predicate::Closure` is `FnMut` and can mutate captured counters               | `trap::predicate_closure_fnmut_counter`                                                                 |
| Predicate-closure errors surface as `EmuError::TrapPredicate`                  | `trap::predicate_closure_error_surfaces_as_emu_error`                                                   |
| `Predicate::Source` raises `TrapPredicateError::SourceRequiresEvaluator`       | `trap::predicate_source_returns_typed_error`                                                            |
| `TrapDispatcher::remove` returns `bool`; `remove_entry` returns public `RemovedTrap` | `trap::remove_returns_bool`, `trap::remove_entry_returns_public_struct`                            |
| `TrapAction::HaltAndReport` halts; `TrapAction::Continue` logs as `NormalizedTraceEvent::TrapHit` | `trap::halt_vs_continue_actions`, `trap::continue_action_logs_normalized_trap_hit`     |
| `AddressRange::new(start, end)` rejects `start > end`                          | `trap::address_range_rejects_inverted`                                                                  |
| `export_persistable_specs` skips closure-only entries with a typed error       | `trap::export_persistable_specs_skips_closures`                                                         |
| `TrapSpec` round-trips through serde                                           | `trap::trap_spec_serde_round_trip`                                                                      |
| `TrapKind` and `TrapAction` round-trip through serde                           | `trap::serde_kind_action_round_trip`                                                                    |
| `MemoryWrite` carries `BankSnapshot` and `TraceOrigin::GuestCpu` for guest writes | `trace_ring::memory_write_carries_bank_snapshot`, `trace_ring::memory_write_origin_guest_cpu`        |
| Host pokes do not produce guest memory-write events                            | `trace_ring::poke_does_not_emit_guest_event`                                                            |
| Two identical consecutive guest writes produce two events (no dedup)           | `trace_ring::two_identical_consecutive_guest_writes_are_two_events`                                     |
| BANK1+BANK2 writes produce `RomBankSwitch` with low/high bit semantics         | `trace_ring::rom_bank_switch_low_byte`, `trace_ring::rom_bank_switch_high_bit`                          |
| RAMB write produces `SramBankSwitch`                                            | `trace_ring::sram_bank_switch`                                                                          |
| IO write produces `IoWrite` with the correct register address                  | `trace_ring::io_write_event`                                                                            |
| `TrapHit` log entries are F-A7-local, not F-A3 typed                           | `trace_ring::trap_hit_event_is_local`                                                                   |
| F-A3 `TraceEvent` passes through as `Typed`                                     | `trace_ring::typed_passthrough`                                                                         |
| `cycle` field is `ClockCycles`                                                  | `trace_ring::cycle_field_is_clock_cycles`                                                               |
| `TraceDropPolicy::DropOldest` drops oldest under pressure                      | `trace_ring::drop_oldest_under_pressure`                                                                |
| `TraceDropPolicy::HaltAndError` raises `EmuError::TraceCapacityExceeded`        | `trace_ring::halt_and_error_on_overflow`                                                                |
| `NormalizedTraceEvent` round-trips through serde for every variant             | `trace_ring::serde_round_trip`                                                                          |
| `Emulator::poll_harness` returns `None` when the doorbell hasn't fired         | `harness::poll_returns_none_when_no_doorbell`                                                           |
| `poll_harness` parses the F-A3 command block                                    | `harness::poll_parses_f_a3_command_block`                                                               |
| `poll_harness` rejects wrong magic with `HarnessMagicMismatch`                 | `harness::poll_rejects_wrong_magic`                                                                     |
| `poll_harness` rejects command-block seq mismatch (not doorbell value)         | `harness::poll_rejects_seq_mismatch_on_command_block_seq`                                               |
| `write_harness_result` writes the F-A3 result-block bytes to `slot.result_addr` | `harness::write_result_writes_result_block_bytes`                                                       |
| Harness reads do not read into the result block region                         | `harness::read_command_does_not_read_result_block`                                                      |
| Harness operations do not emit guest memory-write events                       | `harness::poll_does_not_emit_guest_memory_writes`, `harness::write_result_does_not_emit_guest_memory_writes` |
| Harness operations do not trigger guest memory traps                           | `harness::poll_does_not_trigger_guest_memory_traps`                                                     |
| `HarnessSlot` carries `sram_bank` (MBC5 banking is explicit)                   | `harness::slot_carries_sram_bank`                                                                       |
| `last_seen_seq` advances only on successful read                               | `harness::last_seen_seq_advances_only_on_successful_read`                                               |
| F-A7 introduces no `unsafe`                                                    | `#![forbid(unsafe_code)]` at the crate root (compiler-enforced)                                         |
| Single-substrate boundary is documented (not enforced) until `gbf-test` lands  | `single_substrate_smoke::grep_no_other_gameroy_imports` is `#[ignore]`d; **not** counted as enforcement |
| F-A1's deferred `tiny_rom_boots` test is closed                                | `tests/tiny_rom_boots.rs` exists, uses an explicit `BootMode`, and passes against the checked-in fixture |
| `gameroy-core` is pinned by git rev in `Cargo.toml` and recorded in `Cargo.lock` | `cargo tree -p gbf-emu` shows the exact `gameroy-core` rev; review packet records the pin             |
| `BootMode::BootRom` carries actual 256 boot-ROM bytes (BootRomImage)           | `adapter::boot_rom_constructs_via_gameboy_new`; type-level: `BootRom(BootRomImage { bytes: Box<[u8;0x100]>, sha256: Hash256 })` compiles |
| `Regs.ime` is tri-state `ImeSnapshot`                                          | `primitives::regs_ime_round_trip_to_be_enable`; `primitives::regs_ime_round_trip_disabled_enabled`     |
| `peek` returns `EmuError::DebugMemoryUnsupported` for IO range                  | `adapter::peek_unsupported_for_io_region`                                                              |
| `poke` returns `EmuError::DebugMemoryUnsupported` for IO range                  | `adapter::poke_unsupported_for_io_region`                                                              |
| Memory traps under `io_trace` are post-instruction                              | `trap::mem_watchpoint_state_is_post_instruction`; `trap::mem_watchpoint_*_reports_first_matching_access_after_step` |
| `io_trace` is drained per-`interpret_op`                                       | `trace_ring::io_trace_drained_per_instruction`; `trace_ring::cycle_reconstructed_from_step_clocks`     |
| `TraceOrigin::HostBus` tags `bus_read`/`bus_write` events                      | `trace_ring::bus_write_origin_host_bus`                                                                |
| `AudioOutputMode::Disabled` is the only variant; `Enabled` removed             | Type-level: `match audio { AudioOutputMode::Disabled => () }` is exhaustive                            |
| `PowerOnRamPolicy::FixedFill { wram, hram, cartridge_ram }` is per-region     | `determinism::power_on_ram_fixed_fill_per_region_distinguishes`                                        |
| Loading an RTC cartridge under M0 returns `EmuError::Determinism`              | `determinism::rtc_cartridge_unsupported_returns_typed_error`                                            |
| `SnapshotLineage` carries `boot: BootModeLineage`                              | `snapshot::rejects_boot_mismatch`; `primitives::snapshot_lineage_boot_mode_serde`                       |
| Harness writes do not advance clock and do not emit bank-switch events         | `harness::write_result_does_not_advance_clock`; `harness::write_result_does_not_emit_sram_bank_switch_event` |
| `MemoryAccessKind::InstrFetch` is recognized under `io_trace`                  | `trap::mem_watchpoint_read_includes_instr_fetches_under_io_trace`                                      |

## 17. References

### Internal

- `history/planv0.md` — line 157 (workspace `gbf-emu` slot), line 196 (gameroy-backed primitives + breakpoint/watchpoint trapping; `gbf-debug` lives one crate over), line 207 (six-module layout), line 313 (gameroy as the sole Rust-native core; library API; trap dispatcher; `DeterminismPolicy`), lines 2715–2720 (single-backend rationale), line 2729 (determinism is a `gbf-emu` policy), line 2901 (M0 scope).
- `history/glossary.md` — uses existing terms (`Bead`, `Feature`, `Task`, `Contract`, `Owner`); introduces no new RFC vocabulary.
- `CONSTITUTION.md` — §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silent on success / loud on failure), §VI.1 (single source of truth).
- `.agents/skills/asm-bead-closure/SKILL.md` — closure-skill checklist; the type-boundary, JSON-facing schema, and exhaustive-enum rules apply transitively to F-A7.
- `bd-3mxe` (F-A7 feature bead) and child tasks `bd-1t5d`, `bd-1aql`, `bd-10y1`, `bd-19as`, `bd-14yy`, `bd-16z8`.
- F-A1 RFC `history/rfcs/F-A1-gbf-asm.md` — produces `tiny_rom.gb` + `tiny_rom.sym`; defers `tiny_rom_boots` test to F-A7 (F-A1 §1.2).
- F-A2 RFC `history/rfcs/F-A2-gbf-hw.md` — supplies `Button`, MBC5 register addresses, `MemoryRegion`, `classify(addr)`, `JOYP_REGISTER`, interrupt vectors.
- F-A3 RFC `history/rfcs/F-A3-gbf-abi.md` — supplies `BuildIdentityBlock`, `HarnessCommandBlock`/`HarnessResultBlock` `#[repr(C)]` layouts, `TraceEvent`, `FaultCode::HarnessProtocolError`.
- F-A8 (`bd-1o08`) — `gbf-debug` agent CLI; consumes `Emulator`, `Snapshot`, `TrapDispatcher`, `NormalizedTraceEvent`.
- F-D2 — Harness Control Plane; consumes `HarnessChannel`.
- F-D3 — Trace Pipeline; consumes `NormalizedTraceEvent`.
- F-E2 (`bd-2ww0`) — `PlatformCalibrationBundle` production; consumes `Emulator`, `DeterminismPolicy`, `TrapDispatcher`.

### External

- gameroy emulator workspace (Rust-native backend): <https://github.com/Rodrigodd/gameroy> — dual-licensed `MIT OR Apache-2.0`. F-A7 depends on the **`gameroy-core`** package (under `core/`), not the root `gameroy` GUI/application package.
- gameroy-core `Cargo.toml`: <https://raw.githubusercontent.com/Rodrigodd/gameroy/master/core/Cargo.toml>
- gameroy-core public surface (provisional, pinned by spike): `gameroy::gameboy::GameBoy`, `gameroy::interpreter`, `gameroy::parser`, `gameroy::save_state`. See <https://raw.githubusercontent.com/Rodrigodd/gameroy/master/core/src/lib.rs>.
- mooneye-test-suite (accuracy substrate cited by upstream gameroy): <https://github.com/Gekkio/mooneye-test-suite>
- Pan Docs root: <https://gbdev.io/pandocs/>
- Pan Docs §"MBC3" (RTC reference; MBC5 has none): <https://gbdev.io/pandocs/MBC3.html>
- gekkio CPU manual: <https://gekkio.fi/files/gb-docs/gbctr.pdf>

## 18. Appendix: file-by-file change set

| File                                       | Change                                                            | Lines (est.) |
|--------------------------------------------|-------------------------------------------------------------------|--------------|
| `gbf-emu/src/adapter.rs`                   | New (renamed from `adapters.rs` stub) — replaces stub             | ~320         |
| `gbf-emu/src/primitives.rs`                | New                                                               | ~200         |
| `gbf-emu/src/determinism.rs`               | New                                                               | ~140         |
| `gbf-emu/src/trap.rs`                      | New (renamed from `breakpoints.rs` stub) — replaces stub          | ~220         |
| `gbf-emu/src/trace_ring.rs`                | New (renamed from `trace.rs` stub) — replaces stub                | ~180         |
| `gbf-emu/src/harness.rs`                   | New (replaces stub; same filename)                                | ~160         |
| `gbf-emu/src/lib.rs`                       | Add `#![forbid(unsafe_code)]`, six `pub mod` declarations, doc comment | +25      |
| `gbf-emu/build.rs`                          | New — reads cargo metadata to embed the `gameroy-core` git rev as `const GAMEROY_GIT_REV` | ~40 |
| `gbf-emu/Cargo.toml`                        | Add `gameroy = { package = "gameroy-core", git, rev }` pin; add `thiserror` (and optional `sha2`); move `serde_json` to `[dev-dependencies]`; pin `serde` features; add `build = "build.rs"` | +12 |
| `gbf-emu/tests/gameroy_api_spike.rs`        | New (compile-only spike, §3.7); folded into `adapter.rs` once stable | ~80      |
| `gbf-emu/tests/fixtures/tiny_rom.gb`        | New (golden fixture; binary)                                      | (binary)     |
| `gbf-emu/tests/fixtures/tiny_rom.gb.provenance.toml` | New (SHA-256 + producer command)                          | ~10          |
| `gbf-emu/tests/adapter.rs`                  | New                                                               | ~180         |
| `gbf-emu/tests/primitives.rs`               | New                                                               | ~90          |
| `gbf-emu/tests/determinism_single_path.rs`  | New (load-bearing)                                                | ~110         |
| `gbf-emu/tests/trap.rs`                     | New                                                               | ~190         |
| `gbf-emu/tests/trace_ring.rs`               | New                                                               | ~170         |
| `gbf-emu/tests/harness.rs`                  | New                                                               | ~110         |
| `gbf-emu/tests/snapshot_round_trip.rs`      | New                                                               | ~90          |
| `gbf-emu/tests/tiny_rom_boots.rs`           | New (closes F-A1 deferred test; uses explicit `BootMode`)         | ~70          |
| `gbf-emu/tests/cross_module_conformance.rs` | New                                                               | ~120         |
| `gbf-emu/tests/single_substrate_smoke.rs`   | New (`#[ignore]`; documentation-only until `gbf-test` promotes)   | ~60          |
| `gbf-emu/examples/load_tiny_rom.rs`         | New                                                               | ~50          |
| `Cargo.lock`                                | Updated with `gameroy-core` pin                                   | (mechanical) |

**Total: ~2,500 LOC, ~64% of which is tests, fixtures, and integration coverage.** The spike file and the fixture provenance are the new entries vs. the prior estimate.

## 19. Review packet requirements

The F-A7 PR ships with a **review packet**, but the packet is **scoped to the unresolved technical risk** rather than to a full design dossier. The first review packet exists to prove the hard facts (gameroy-core pin, adapter API, determinism hash, fixture hash) — diagrams and rendered SVGs are useful but not required for the first merge unless reviewers request them. The packet is authored *after* implementation, so the §3.7 adapter spike's findings are reflected in the packet's API guide.

### 19.0 Minimum packet for merge

Required artifacts in the first review packet:

- **gameroy-core pin record**: exact `gameroy-core` git rev pinned, the `Cargo.lock` excerpt, the rationale for pinning by git rev (not SemVer), and the bump policy.
- **Adapter API proof**: the §3.7 adapter spike compiles green; a one-page summary lists the actual public-API paths used (construction, step, save-state, access observation, framebuffer, joypad).
- **Determinism hash**: the SHA-256 of `(framebuffer, regs, mem_dump)` after the canonical `tests/determinism_single_path.rs` run, recorded so a reviewer can compare against their own checkout.
- **`tiny_rom` fixture hash**: the SHA-256 of `gbf-emu/tests/fixtures/tiny_rom.gb` plus the F-A1 example version that produced it and the regeneration command.
- **Claim-to-gate matrix**: every load-bearing claim from §16 mapped to its gating test, file, or artifact.
- **`cargo test -p gbf-emu` transcript**: full output of the local test run, with a sub-second timestamp per test for reviewer comparison.
- **Known-debt ledger**: every TODO, FIXME, deferred decision, or known-imperfect aspect introduced or carried by F-A7, with the bead/feature that owns the resolution. Includes the workspace-wide grep gate promotion (`#[ignore]` smoke → `gbf-test` workspace gate), any gameroy-specific behaviors that needed to be worked around, and the "spike folded into adapter.rs" status.

### 19.1 What the packet must let the reviewer do

A reviewer who is otherwise unfamiliar with F-A7 should be able to answer four questions in one sitting:

1. **Is the implementation correct?** — gameroy wrap is faithful, determinism is provably byte-equal across runs, traps fire when expected, harness blocks parse against F-A3 layouts.
2. **Is it clear and maintainable?** — Single substrate, no other crate imports gameroy, names match the design surface this RFC commits to.
3. **Are the riskiest invariants actually proved?** — By tests, by type structure, or by the gameroy pin — not by prose alone.
4. **Can I reproduce every claimed output locally?** — Tests, generated artifacts, dependency reports, and cross-crate workspace effects.

### 19.2 Recommended topics (beyond §19.0's required minimum)

In addition to the §19.0 minimum, the packet *may* cover the following when reviewers request them or when the implementation surfaced something worth recording:

- **Scope statement** — what is in scope for this PR (six modules + tests + example + spike), what is intentionally deferred (F-A8 / F-D2 / F-D3 / F-E2), and which downstream feature picks up each deferred item.
- **Reading order** — recommended sequence: which file or topic to read first, which to read deeply, which to skim, which to ignore.
- **Diff disposition** — for every file in the PR diff, a one-line classification (deep review / boundary review / skim / mechanical re-export / generated / fixture / config). Exhaustive over `gh pr diff --name-only`.
- **Architecture brief** — how `gbf-emu` decomposes (the six modules), why each module is where it is, and what the dependency direction is. Reuses material from §3.
- **Correctness dossier** — for each of the highest-risk surfaces (determinism single-path; snapshot lineage rejection; trap dispatcher PC-trap-fires-before-instruction correctness; trace normalization for BANK1/BANK2; harness magic-byte parsing; F-A1 boot test), the packet records the invariant, the test or type or citation that proves it, and the failure mode if it ever drifts.
- **Test coverage report** — what `cargo test -p gbf-emu` runs, how it groups, what it asserts, and any portions deliberately not covered (with reasoning).
- **Reproducibility report** — the exact command set a reviewer runs to regenerate every checked-in artifact (test output, dependency tree, gameroy pin verification). One top-level script invocation should reproduce all of it.
- **Generated artifacts manifest** — what artifacts (test logs, cargo trees, framebuffer hashes) ship with the packet, and a reproducibility-fingerprint per artifact.
- **Dependency report** — `cargo tree -p gbf-emu` plus an explicit confirmation that no `[dev-dependencies]` on `gbf-bench`, `gbf-runtime`, `gbf-codegen`, or training-side crates leak in. Includes the `gameroy-core` license verification (`MIT OR Apache-2.0`).
- **Out-of-scope ledger** — items that look like F-A7's job but explicitly are not (HarnessOp dispatch, scripting host, session file format, calibration bundle production, mooneye gates), each with the owning feature.
- **API guide** — the public surface of each module, the value objects (with their invariants), the error variants, and the (pre-1.0) source-break policy.
- **Reviewer checklist** — the binary questions the reviewer should be able to mark off (e.g., "every consumer goes through `Emulator`", "`DeterminismPolicy::default()` is locked-down", "no other crate imports `gameroy-core`", "F-A1's `tiny_rom.gb` boots within `MAX_TINY_ROM_BOOT_BUDGET`").
- **Cleanliness audit** — confirmation that `#![forbid(unsafe_code)]` is at the crate root, that no `bytemuck` or `transmute` was introduced, that `serde_json` stays in `[dev-dependencies]`, and that no thread-spawn primitives were added.
- **F-A1 follow-up evidence** — confirmation that `tests/tiny_rom_boots.rs` exists and passes, with the exact `tiny_rom.gb` SHA-256 it consumed and the explicit `BootMode` it used; the F-A1 §16 deferred row is now closed in this PR.
- **Diagrams** — module dependency diagram, the `Emulator` state-machine sketch (idle ↔ running ↔ trap-hit), the trace normalization pipeline (access observation → cursor → drain), and the harness channel flow (doorbell edge ↔ command-block seq ↔ result-block write). Diagrams are **optional for the first merge**; they ship in the packet only when reviewers ask for them or when the implementation surfaced a non-obvious flow worth visualizing. Mermaid sources plus rendered SVGs; the rendering is reproducible.

The packet may add other sections that turn out to be useful at implementation time, but the §19.0 minimum is non-negotiable.

### 19.3 Reproducibility property

The packet contains a single top-level `verify-packet` script (or equivalent). Running it in a fresh checkout regenerates every artifact-the-packet-references and fails loudly if any checked-in artifact is stale relative to the current source. The exact script name, location, and output format are decided at packet-creation time; the contract is that one command suffices.

### 19.4 Acceptance bar

The packet is complete only when:

- a fresh-checkout reviewer can run the verify script, all tests pass, all reproducible artifacts match;
- every claim in §16 (claim-to-gate matrix) maps to a concrete gate (test, type, citation, or generated artifact);
- every file in the PR diff appears exactly once in the diff disposition table;
- the gameroy pin is verifiable from `Cargo.lock`;
- the determinism single-path test passes byte-equally across two consecutive runs;
- F-A1's `tiny_rom_boots` test exists and passes;
- the cleanliness audit shows zero introduced uses of disallowed APIs;
- known-debt and out-of-scope ledgers are present and entries point at owning beads or RFCs.

### 19.5 What the packet should explicitly not pre-commit to in *this* RFC

This RFC deliberately stops short of specifying packet directory structure, file names, README templates, exact diff-map columns, or the exact reviewer-question wording. The reasons:

- The packet describes the *implementation that actually shipped*. Pre-committing to a fixed structure here would force the implementation to match an a-priori shape that may not fit what was built.
- Implementation decisions (e.g., a redesign of an `EmuError` variant, a reorganisation of test fixtures, an additive helper that didn't exist when this RFC was drafted) need to be reflected in the packet without first amending this RFC.
- The packet is itself a deliverable; locking its structure here makes the RFC into a packet template, which is the wrong level of abstraction.

The reviewer asks under §20 are part of *this* RFC's deliverable, not the packet's; they remain in this document.

## 20. End

This RFC stays inside the F-A7 boundary. Anything that requires F-A8's scripting host, F-D2's harness control-plane semantics, F-D3's trace transport ring, F-E2's calibration bundle production, or `gbf-test`'s workspace-wide enforcement is explicitly deferred. The proposal lets F-A7 close without those features existing, while leaving every seam (`Emulator`, `Snapshot`, `TrapDispatcher`, `Predicate::Source`, `NormalizedTraceEvent::Typed`, `HarnessChannel`, `DeterminismPolicy::fingerprint()`) shaped for them to plug in cleanly.

Reviewer asks I would value most (numbered in priority order):

1. **Does the pinned `gameroy-core` revision actually expose the construction / step / save-state / framebuffer / cartridge-loading / access-observation API surface this RFC assumes?** The §3.7 adapter spike answers this; if the answer is "no" for any of the six, which assumptions in §4–§8 change? This is the single most important review question — most of the RFC depends on the answer.
2. Is the **split between `bus_read`/`bus_write` (side-effecting) and `peek`/`poke` (side-effect-free)** at the right granularity? Specifically, do trap predicates and harness pokes need any other access verb, or are these two enough? Gameroy-core's interior-mutability behavior on memory reads is the load-bearing reason for the split.
3. Is the **`Emulator` verb set** at the right granularity? Specifically, is `run_until_pc` the right shape, or is `run_until { pc, mem_predicate, max_cycles }`-style needed at M0? Adding verbs is intentional source-break pre-1.0; getting the M0 set right at closure time avoids one round of consumer churn.
4. Is the **`Predicate` enum shape** right? F-A7 commits to `Always` / `Closure(FnMut(&TrapContext) -> Result<bool, _>)` / `Source(String)` with `Source` opaque to F-A7 and F-A8 compiling it into `Closure` before dispatch. The alternative — a single `Source(String)` variant with F-A7 owning a JS evaluator — would simplify F-A8 at the cost of pulling rquickjs into `gbf-emu`. Cross-check welcome.
5. Is the **`NormalizedTraceEvent` set** at M0 the right closure? F-A7 commits to `MemoryWrite` (with `bank` + `origin`) / `RomBankSwitch` / `SramBankSwitch` / `IoWrite` / F-A7-local `TrapHit` / F-A3 `Typed` passthrough. Should `JoypadInput` or `InterruptFire` be included now, or is the `Typed` passthrough sufficient because the runtime nucleus emits typed events for those?
6. Is the **`DeterminismPolicy::default()` lockdown** the right shape? Specifically: is the cartridge RTC pin at `2000-01-01T00:00:00Z` the right anchor (no-op for MBC5, live for future MBC3); should `PowerOnRamPolicy` exist at all (or is the pinned gameroy-core fully deterministic without it); and is `AudioOutputMode::Disabled = no host sink` the right "audio-off" semantic vs. "APU off"?
7. Is closing the **F-A1 `tiny_rom_boots` test** in F-A7 the right home? F-A1 §1.2 deferred to "the follow-up `gbf-emu`/`gbf-debug` feature" without specifying which one. F-A7 picks it up because (a) `gbf-debug` (F-A8) is downstream of F-A7, and (b) the test is structural (boot reaches `$0150`), not scripted (which would be F-A8's home). If the boot test should live in F-A8 instead, that is a half-day move.
