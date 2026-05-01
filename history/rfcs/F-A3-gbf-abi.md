# RFC F-A3: `gbf-abi` — Live Execution Contract Types

| Field          | Value                                                              |
|----------------|--------------------------------------------------------------------|
| Author         | bkase (engineer picking up F-A3)                                   |
| Status         | Draft                                                              |
| Feature bead   | `bd-2k2`                                                           |
| Open tasks     | T-A3.1 (version), T-A3.2 (continuation+liveness), T-A3.3 (harness), T-A3.4 (fault), T-A3.5 (interrupt+lease), T-A3.6 (checkpoint), T-A3.7 (trace) |
| Closed tasks   | (none — gbf-abi crate is module-stub-only at start of F-A3)        |
| Plan reference | `history/planv0.md` line 271 (gbf-abi ownership), line 1795 (liveness), line 2170 (auto-yielding ABI), line 2533/2552 (identity handshake), line 2554 (fault domains + harness control plane) |
| Constitution   | `CONSTITUTION.md` §I.1 (correctness by construction), §III (shifting left), §IV.3 (reproducible builds), §V.3 (silence on success, loud on failure), §VI.1 (single source of truth) |

## Project context — where F-A3 sits in the plan

This RFC is small in line count and large in blast radius. Before any of the type definitions in §3 make sense, a reader needs to know what `gbf-abi` is *for* in the project as a whole and where it sits inside Epic A specifically. This preface answers both questions; the numbered sections that follow are the design itself.

### The whole-project framing

GBLLM is an end-to-end Rust toolchain for compiling and running small language-model inference on a stock Game Boy DMG cartridge. `planv0.md` (line 1 onward) frames the system as **five cooperating products plus three shared contracts**:

- **Five products** (an ownership/build decomposition): a Burn-fronted Rust training stack; a frozen artifact/export boundary plus a separate `CompileRequest` boundary; a three-layer oracle stack (`DenotationalOracle`, `ArtifactOracle`, `ScheduleOracle`); a staged compiler; a cooperative Game Boy runtime.
- **Three shared contracts** that the five products meet at, kept as their own crates so semantic drift between subsystems is structurally impossible:
  - `gbf-hw` — the **target contract** (cartridge profile, physical constraints, calibration schema).
  - `gbf-artifact` — the **durable model contract** (`ArtifactCore`, `TargetDataLoweringArtifact`, `ArtifactAux`, optional `ReferenceModelBundle`).
  - `gbf-abi` — the **live execution contract**: every type that crosses a process/process or process/ROM boundary at run time.

`gbf-abi` is the third bullet, and that is the entire reason this RFC exists. Without it, every consumer of run-time state — the Bank0 cooperative runtime, the harness control plane, the gbf-emu adapter, the three oracles, the persistence layer, the failure-capsule reporter — would be free to invent its own `InferenceState`, its own fault taxonomy, its own checkpoint id, its own trace event layout. The job of `gbf-abi` is to make exactly one definition of each of those types exist in the workspace, behind compile-time layout assertions and exhaustive enums, so that "compiler/runtime/harness/emulator/oracles agree about run-time state" stops being a hope and becomes a `cargo check` invariant.

### Where F-A3 sits inside Epic A

Epic A (`bd-14y`) is the **M0 foundation stack** — the first milestone in `planv0.md` §"What I would build first" (line 2897). M0's deliverable is a Game Boy that boots a deterministic, agent-debuggable ROM with a cooperative runtime skeleton, an emulator harness that talks to it, and the typed contracts every later milestone will compile against. The Epic A features and how they relate to F-A3:

| Feature | Bead | What it delivers | Relationship to F-A3 |
|---------|------|------------------|----------------------|
| F-A1 | `bd-ssm` | `gbf-asm` typed LR35902 eDSL — the only legal authoring layer for Game Boy bytes | Sibling; in progress (PRs 1–3 merged). Independent of F-A3. |
| F-A2 | (gbf-hw) | DMG/MBC5 target profiles + calibration schema | Sibling; F-A3 does not depend on it. |
| **F-A3** | **`bd-2k2`** | **`gbf-abi` — this RFC. Live execution contract types.** | **Owns the typed vocabulary the rest of Epic A talks in.** |
| F-A4 | `bd-1sv` | Real `BankLease`/`BankGuard` runtime ABI | Consumes F-A3's `ResourceLease` / `ResourceLeaseKind` / `InterruptPolicy`. |
| F-A5 | `bd-2r1` | Bank0 cooperative runtime nucleus | Allocates `InferenceState` in WRAM, raises `FaultCode::LivenessTimeout`, polls the `HarnessCommandBlock` doorbell, writes the `BuildIdentityBlock` at boot. |
| F-H3 | `bd-1aqc` | gbf-emu adapter + gbf-debug agent CLI | Reads `BuildIdentityBlock`, `HarnessCommandBlock`/`HarnessResultBlock`, `TraceEvent` from a memory-mapped buffer using F-A3's pinned layouts. |

F-A3 is **upstream of F-A4, F-A5, and F-H3 inside Epic A**: those features need typed names for leases, faults, control-plane commands, and trace events before they can implement the algorithms that operate on them. F-A3 ships only the type definitions, the layout proofs, the exhaustive enums, the parsers, and the negative tests; the runtime, banking, and emulator features then pick up those types and write the actual scheduling, banking, polling, and tracing code. (The full downstream consumer list, including the oracle stack F-C1/F-C2/F-C3, the persistence pipeline F-D1, the harness runtime F-D2, the trace transport F-D3, the fault-policy table F-D5, the build-report family F-F1/F-F2, and the failure-capsule reporter F-F4, is enumerated in §2.3.)

### Why this contract has to land before the rest of M0

`planv0.md` repeatedly identifies the same architectural risk (line 1795, 2170, 2398, 2533, 2552, 2554): a cooperative single-threaded Game Boy runtime can be locally safe at every yield point and still globally broken — livelocked, oscillating between tiny slices with no semantic progress, repeatedly revisiting the same checkpoint, starving generation while the UI stays smooth. The plan's response is two non-optional rules:

1. **The liveness contract is not optional.** Every slice must participate in `progress_epoch` / `last_checkpoint` / `no_progress_frames` accounting, and `FaultCode::LivenessTimeout` is a first-class typed failure mode, not an ad hoc panic.
2. **The identity handshake is not optional.** Compiler, runtime, harness, and emulator must all agree on which build they are looking at, via a four-hash `BuildIdentityBlock` (build / artifact-core / runtime-nucleus / compile-request) that is queryable at a known ROM offset.

F-A3 is where both rules become *types*: `LivenessCounters` is a non-Option field of `InferenceStateHeader`; `BuildIdentityBlock` is `#[repr(C)]` with pinned offsets and four lineage hashes; the oracle stack indexes everything by `SemanticCheckpointId` rather than by raw addresses. Once those types exist, downstream features cannot accidentally ship a runtime that lacks liveness accounting, a harness that talks to the wrong build, or an oracle that names checkpoints differently from the runtime that implements them. This is what the constitution calls "shifting left" (§III): correctness moves from runtime checks into compile-time invariants.

In short: F-A3 is the small, foundational crate that converts the planv0.md liveness and identity rules from documentation into the type system, and it is the prerequisite for the cooperative runtime, the harness control plane, the emulator adapter, every oracle, every report, and the persistence layer. Everything else built on top of M0 inherits its guarantees from this RFC.

## 0. TL;DR

`gbf-abi` is the **live execution contract** between the compiler, runtime, harness, emulator adapters, and `ScheduleOracle`. It is `gbf-artifact`'s sibling: where `gbf-artifact` owns the durable offline model lineage, `gbf-abi` owns every type that crosses a process/process or process/ROM boundary at run time. It is the natural home for `#[repr(C)]` layouts and the only place we put compile-time layout assertions for types that the emulator, debugger, harness, or ROM-resident structures must agree on bit-for-bit.

The crate ships eight modules — `version`, `continuation`, `liveness`, `harness`, `fault`, `interrupt`, `checkpoint`, `trace` — totalling roughly 1,200–1,500 LOC of production code plus ~1,600 LOC of layout, serde, exhaustive-enum, schema, and negative-input tests. Every type that is ever read by anything other than gbf-abi itself (host tooling, runtime polling code, ROM-resident structures) gets a pinned `static_assertions::const_assert_eq!(size_of::<T>(), N)` and per-field `offset_of!` assertion, plus a JSON round-trip for the host-only types.

The seven most load-bearing decisions in this RFC are:

1. **Layout stability is enforced at compile time.** Every `#[repr(C)]` struct has both a `const_assert_eq!(size_of::<T>(), N)` and a `const_assert_eq!(memoffset::offset_of!(T, f), F)` per field. A field reorder cannot ship without a compile error. Discriminants for `#[repr(u16)]` enums are pinned with explicit values and an exhaustive `ALL` array test.
2. **`gbf-abi` is `no_std + alloc` capable.** Modules whose types are ROM-resident (the `BuildIdentityBlock`, `InferenceState` prefix, `HarnessCommandBlock`/`HarnessResultBlock`, `FaultSnapshot`, `TraceEvent`) use only `Copy` POD-shaped fields with no allocation. Host-only types (`CompatibilityEnvelope`, `SemanticCheckpointSchema`, `FaultPolicy`) are gated behind the `host` feature and may use `alloc::vec::Vec`, `alloc::string::String`, and `alloc::borrow::Cow`. F-A3 does not derive or implement `bytemuck::Pod` inside `gbf-abi`. The crate keeps `#![forbid(unsafe_code)]` as an absolute local invariant and provides byte parsing through explicit little-endian `from_le_bytes` / `to_le_bytes` helpers only.
3. **`SemanticCheckpointId` is the only durable id; `CompactCheckpointId` is its build-local compression.** The mapping lives in an exported `SemanticCheckpointSchema` (sibling of `gbf-artifact::ArtifactAux`-style export) consumed by harness, oracles, runtime, reports. No subsystem may invent a compact id; only the schema mints them, and they are unique within a build.
4. **The continuation has a stable prefix and a build-specific tail.** `InferenceState` carries an explicit fixed-size prefix containing `abi`, `schema`, `session_id`, `token_count`, `slice_id`, `liveness`, and `last_fault`. Whatever bytes follow in WRAM are opaque to `gbf-abi`; they belong to per-build sequence-state. Layout assertions cover the prefix only. The tail length is recorded in `BuildIdentityBlock::continuation_tail_bytes`.
5. **Liveness counters are typed first-class state, not commentary.** `LivenessCounters` carries `progress_epoch` (monotone u32), `last_checkpoint` (CompactCheckpointId), `no_progress_frames` (u16 with saturating arithmetic), `livelock_threshold_frames` (u16). `record_progress`, `note_idle_frame`, and `is_livelocked` are methods, not free helpers. `FaultCode::LivenessTimeout` and `FaultCode::RepeatedCheckpointNoProgress` are typed sibling failure modes.
6. **`FaultCode` is exhaustive and partitioned by `FaultDomain`.** Discriminants are pinned (e.g. `0x0010..=0x001F` for persistence, `0x0020..=0x002F` for banking) so that report consumers can range-check without a lookup table. `classify_fault(FaultCode) -> FaultDomain` is a total function tested for every `FaultCode::ALL` member.
7. **Harness command/result blocks are a bidirectional control plane, not a result tape.** Each block carries a magic header (`"HCMD"` / `"HRES"`), a monotone `seq` matched between command and result, an op/kind discriminator, and a 32-byte payload. The `HarnessOp` enum covers `Nop`, `StepSlice`, `RunUntilCheckpoint`, `DumpArena`, `InjectFault`, `PowerCut`, `SetSession`, `GetState` — exactly the planv0.md "real control plane" set.

## 1. Goals and non-goals

### 1.1 Goals (in scope for this RFC)

- A pinned `AbiVersion` semver triple plus a `CURRENT_ABI` constant that any subsystem can compare against.
- A `CompatibilityEnvelope` that declares the past versions a current build remains compatible with, plus a forward-handshake list for tolerated future versions.
- A `#[repr(C)]` `BuildIdentityBlock` with `"GBLM"` magic, the four lineage hashes, and a build timestamp; ROM-resident at a known address (the address itself is the runtime/compiler's contract — `gbf-abi` only owns the layout).
- A `#[repr(C)]` `InferenceState` prefix with a stable size and a build-specific opaque tail. Layout asserted at compile time; tail length recorded in `BuildIdentityBlock`.
- Typed `LivenessCounters` with monotone progress accounting, saturating idle-frame accumulation, and a `FaultCode::LivenessTimeout` raise rule.
- `HarnessCommandBlock` / `HarnessResultBlock` with magic bytes, monotone sequence numbers, and a complete `HarnessOp` set (`Nop`, `StepSlice`, `RunUntilCheckpoint`, `DumpArena`, `InjectFault`, `PowerCut`, `SetSession`, `GetState`).
- `FaultCode` enumeration partitioned by `FaultDomain`, with a total `classify_fault` function. `FaultSnapshot` carries enough state (PC, bank, checkpoint, regs, liveness) to debug after the fact.
- `FaultPolicy` mapping `FaultDomain` to `RecoveryAction`, plus `BootValidationPlan` for boot-time validation choices.
- `InterruptPolicy` (`Enabled` / `ShortCriticalSection` / `Disabled`), `ResourceLease` with `ResourceLeaseKind` covering `RomWindow`, `SramPage`, `Overlay`, `InterruptMask`.
- `SemanticCheckpointId` (durable, dotted name) and `CompactCheckpointId` (build-local u16). `SemanticCheckpointSchema` mapping host-side, with stratum classification (`Denotation` / `Artifact` / `Operational`).
- `TraceEvent` with `TraceProbeId`, `ProbeLevel`, `ProbeBudgetClass`. `TraceBudget` with `TraceDropPolicy` (`DropOldest`, `DropNewest`, `HaltAndFault`).
- Layout assertions for every `#[repr(C)]` struct: total size, per-field offset, alignment, magic-byte prefix (where applicable).
- Serde round-trip tests for every host-side type. Negative tests for malformed inputs (out-of-range discriminants, missing required fields, magic-byte mismatch).
- A consumer adapter shape (`SemanticCheckpointSchema::resolve_compact`, etc.) that the oracles and harness can use without depending on the runtime.

### 1.2 Non-goals (deferred)

- **The actual `gbf-runtime::persistence` SRAM record protocol.** F-D1 / T-A6.x. `gbf-abi` declares the on-cartridge field shapes for harness and `BuildIdentityBlock`; the page rotation, CRC verification, and `PersistKind` taxonomy live in F-D1.
- **The full F-A4 `BankLease`/`BankGuard` ABI.** F-A4 (`bd-1sv`) implements the real runtime banking API. F-A3 only declares `ResourceLeaseKind::RomWindow(RomWindowBinding)` so `ResourceStateValidation` (Epic B Stage 10.5) can refer to it; the actual call/return sequence and HRAM shadow registers belong to F-A4.
- **`SemanticCheckpointSchema` *production* (i.e. who emits it and where).** F-A3 owns the type and round-trip. The build pipeline that emits `semantic_checkpoint_schema.json` lives in F-F1 (build reports) and the compiler stage that mints the compact ids lives in `gbf-codegen`.
- **Tracing transport.** F-D3 (`bd-1zxn`, Trace Pipeline) owns the SRAM ring buffer, framing, and harness-side reader. F-A3 owns only the wire-format layout and budget types.
- **Trace event payload schemas.** F-A3 owns the 16-byte fixed payload region of `TraceEvent`. The per-probe interpretation of those bytes is a consumer-side concern (and may be schema-versioned independently of the ABI version itself).
- **`gbf-emu` adapter glue.** F-H3 reads `TraceEvent` / `BuildIdentityBlock` / `HarnessCommandBlock` from a memory-mapped buffer; F-A3 only ships the layout types.
- **CGB / GBC features.** DMG-only. The interrupt mask vocabulary (`VBlank`, `LcdStat`, `Timer`, `Serial`, `Joypad`) is the DMG five.
- **`unsafe` interior reinterpretation.** Host-side parsing of ROM-resident layouts inside `gbf-abi` uses hand-written `from_le_bytes` helpers, not `transmute` and not `bytemuck` derives. F-A3 ships zero `unsafe` lines and keeps `#![forbid(unsafe_code)]` as a hard local invariant. Downstream consumers may choose `bytemuck` for their own adapters; that is outside the F-A3 crate-local safety contract.
- **Schema migration.** F-A6 (`gbf-migrate`) owns the host-side upgrade DAG. F-A3 just bumps `AbiVersion` and bumps the schema version inside `SemanticCheckpointSchema` on breaking changes.
- **Persistence semantic-state-hash separation.** Splitting `semantic_state_hash` / `resume_abi_hash` / `build_identity_hash` per `PersistKind` is F-D1's job; F-A3 only carries the four lineage hashes inside `BuildIdentityBlock`.

## 2. Background and existing state

### 2.1 What is already in tree

`gbf-abi/src/` exists as a module-stub crate at the start of F-A3:

```
gbf-abi/src/
  lib.rs          (10 lines: pub mod declarations only)
  checkpoint.rs   (1 line stub)
  continuation.rs (1 line stub)
  fault.rs        (1 line stub)
  harness.rs      (1 line stub)
  interrupt.rs    (1 line stub)
  liveness.rs     (1 line stub)
  trace.rs        (1 line stub)
  version.rs      (1 line stub)
```

The crate should keep its dependency surface minimal. `gbf-foundation` is used for `SemVer` conversion in §3.1.1; if `gbf-foundation::Hash256` is available as a stable transparent newtype over `[u8; 32]` it may be used inside `BuildIdentityBlock`, otherwise the ABI stores hashes as raw `[u8; 32]` digest bytes. `gbf-abi` does **not** depend on `gbf-artifact`, `gbf-hw`, `gbf-runtime`, `gbf-codegen`, or `gbf-asm` — those are consumers or siblings, not prerequisites of the live ABI contract. F-A3 adds `static_assertions` and `memoffset` as compile-only dependencies; it intentionally does not depend on `bytemuck` (see §3.8 / §4.2).

`gbf-foundation::SemVer` already exists with `major: u64, minor: u64, patch: u64`. **F-A3 does *not* reuse it for `AbiVersion`**: `AbiVersion` is `#[repr(C)]` with `u8` fields because (a) it is ROM-resident inside `BuildIdentityBlock` and must have stable size 3 bytes, and (b) the ABI's version space is bounded — major/minor/patch above 255 has no operational meaning. `gbf-foundation::SemVer` remains the right type for crate versions and other host-only context. The two coexist; `AbiVersion::from_semver(SemVer) -> Option<Self>` performs the bounded conversion.

### 2.2 What does *not* yet exist

- All concrete types in §3.1–§3.8 below.
- The `host` Cargo feature gating `Vec`/`Cow`-using types.
- Any layout assertion or static-assertion test.
- Any adapter trait used by harness or oracles.

### 2.3 Downstream pressure on this design

`gbf-abi` is consumed by:

- **F-A4** (`bd-1sv`, BankLease ABI) — `ResourceLease` / `ResourceLeaseKind` / `InterruptPolicy` are the type vocabulary the BankLease lifecycle is described in.
- **F-A5** (`bd-2r1`, Bank0 runtime) — boot writes a `BuildIdentityBlock` to the known offset. The cooperative scheduler allocates `InferenceState` in WRAM, raises `FaultCode::LivenessTimeout` when liveness fails, and polls the `HarnessCommandBlock` doorbell.
- **F-B11** (`bd-9ae`, GbSchedIR + ResourceStateValidation, Stage 10.5) — `ResourceLease` balance is the validation theorem.
- **F-B13** (`bd-18d`, backend) — emits the cartridge-resident `BuildIdentityBlock`, allocates the WRAM frame for `InferenceState`, and produces `semantic_checkpoint_schema.json`.
- **F-C1 / F-C2 / F-C3** (oracle stack) — `SemanticCheckpointId`, `SemanticStratum`, `CompactCheckpointId` are how the three oracles point at the same checkpoints. `F-A3` is a hard predecessor for all three (it is on the `blocks` edge in beads).
- **F-D1** (persistence) — `BuildIdentityBlock` hashes flow through to per-record-kind `build_identity_hash` fields.
- **F-D2** (harness control plane) — implements the host side of `HarnessCommandBlock` / `HarnessResultBlock`.
- **F-D3** (trace pipeline) — implements the host side of `TraceEvent` ingestion against the layout.
- **F-D5** (`bd-3ot1`, Fault Policy Table + Recovery + BootValidationPlan) — owns the production `FaultPolicy` content; F-A3 owns the *types* it is built from.
- **F-F1 / F-F2** (build reports / certificates) — emit `semantic_checkpoint_schema.json`, `boot_validation.json`.
- **F-F4** (`bd-2cny`, FailureCapsule) — `FaultSnapshot` is the structural minimum a capsule contains.
- **F-H3** (`bd-1aqc`, gbf-emu adapter) — parses ROM-resident `BuildIdentityBlock`, `HarnessCommandBlock`/`HarnessResultBlock`, `TraceEvent` from a memory-mapped buffer using these layouts.
- **T2.5** (runtime nucleus drift gate) — diff `runtime_nucleus_hash` across builds.

### 2.4 Plan and constitution grounding

This RFC threads several plan rules tightly:

- **planv0.md line 271**: gbf-abi is the live-execution contract crate, complementing gbf-artifact (durable model lineage). → §3 is partitioned exactly by that boundary.
- **planv0.md line 1795 / 2170 / 2398**: the liveness contract is not optional. → `LivenessCounters` is non-Option in `InferenceState`; `FaultCode::LivenessTimeout` and `FaultCode::RepeatedCheckpointNoProgress` are first-class.
- **planv0.md line 2533 / 2552**: identity handshake. → `BuildIdentityBlock` carries four hashes and is queryable by harness/emulator.
- **planv0.md line 2554** revision-pass item (8): "turn harness mode into a real control plane". → `HarnessOp` is the full eight-op set, not a single "step" placeholder.
- **planv0.md line 2554** revision-pass item (7): fault domains + graded recovery actions. → `FaultDomain` and `RecoveryAction` are explicit.

Constitutional grounding:

- **§I.1 (correctness by construction)** — `FaultCode`, `FaultDomain`, `HarnessOp`, `HarnessResultKind`, `ProbeLevel`, `ProbeBudgetClass`, `TraceDropPolicy`, `InterruptPolicy`, `ResourceLeaseKind`, `RecoveryAction`, `SemanticStratum` are exhaustive enums with pinned discriminants.
- **§III (shift left)** — layout invariants are compile-time assertions, not runtime checks. A field reorder fails `cargo check`.
- **§IV.3 (reproducible builds)** — every host-side type round-trips through serde without losing information. ROM-resident types serialize to a fixed byte sequence given fixed inputs.
- **§V.3 (silence on success, loud on failure)** — `FaultSnapshot` carries register snapshot, PC, bank, checkpoint, and liveness. Harness-protocol mismatches surface as `FaultCode::HarnessProtocolError` with a sequence number, not as silent corruption.
- **§VI.1 (single source of truth)** — every type defined here has exactly one definition; any `gbf-runtime` / `gbf-codegen` shadow re-implementation is a bug.

## 3. Module-by-module design

This section walks every module that ships in F-A3. Each subsection corresponds to a child task: §3.1 → T-A3.1, §3.2 → T-A3.2, etc.

### 3.1 `version.rs` — `AbiVersion`, `CompatibilityEnvelope`, `BuildIdentityBlock` (T-A3.1)

#### 3.1.1 `AbiVersion`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct AbiVersion {
    pub major: u8,
    pub minor: u8,
    pub patch: u8,
}

pub const CURRENT_ABI: AbiVersion = AbiVersion { major: 0, minor: 1, patch: 0 };

impl AbiVersion {
    pub const fn new(major: u8, minor: u8, patch: u8) -> Self { Self { major, minor, patch } }
    pub const fn is_zero(self) -> bool { self.major == 0 && self.minor == 0 && self.patch == 0 }
    pub fn from_semver(v: gbf_foundation::SemVer) -> Option<Self>;
    pub fn to_semver(self) -> gbf_foundation::SemVer;
}
```

`AbiVersion` is `#[repr(C)]` because it is embedded inside `BuildIdentityBlock` at byte offset 4. The packed size is 3 bytes; `BuildIdentityBlock` does not require any per-field alignment beyond `align_of::<u64>() == 8`, which is satisfied by the layout in §3.1.3.

`CURRENT_ABI` is non-zero by construction so that uninitialized memory (all zeros) cannot accidentally satisfy a version check. A test asserts `!CURRENT_ABI.is_zero()`.

`AbiVersion` is comparable: `(0,1,0) < (0,2,0) < (1,0,0)`. The `PartialOrd` derive over `(major, minor, patch)` produces the right order because `#[derive]` lex-orders fields top-to-bottom.

#### 3.1.2 `CompatibilityEnvelope`

```rust
#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityEnvelope {
    pub current: AbiVersion,
    pub backward_compatible_with: Vec<AbiVersion>,
    pub forward_handshake: Vec<AbiVersion>,
}

#[cfg(feature = "host")]
impl CompatibilityEnvelope {
    pub fn current_only(current: AbiVersion) -> Self;
    pub fn accepts(&self, peer: AbiVersion) -> bool;
    pub fn validate(&self) -> Result<(), CompatibilityError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityError {
    DuplicatesCurrent { duplicate: AbiVersion },
    BackwardLargerThanCurrent { offender: AbiVersion },
    ForwardSmallerThanCurrent { offender: AbiVersion },
    DuplicateInBackward { offender: AbiVersion },
    DuplicateInForward { offender: AbiVersion },
}
```

Validation rules:

- `current` does not appear in `backward_compatible_with` (would be a no-op self-reference).
- Every entry in `backward_compatible_with` is strictly less than `current`.
- Every entry in `forward_handshake` is strictly greater than `current`.
- No duplicates within either list.

`CompatibilityEnvelope` is `#[cfg(feature = "host")]` because it uses `Vec<AbiVersion>` which is not `repr(C)` and not `no_std + alloc`-portable across host/target boundaries. The runtime-side compatibility check uses a fixed-size `BuildIdentityBlock::accepts(peer_block) -> bool` that walks a small, statically-sized table of accepted versions. The host-side `CompatibilityEnvelope` is the *source* of that table; it is baked into the runtime image at compile time.

#### 3.1.3 `BuildIdentityBlock`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildIdentityBlock {
    pub magic:                    [u8; 4],   // b"GBLM"
    pub abi:                      AbiVersion,    // 3 bytes
    pub _reserved0:               u8,            // pad to 8-byte align for hashes
    pub build_hash:               [u8; 32],
    pub artifact_core_hash:       [u8; 32],
    pub runtime_nucleus_hash:     [u8; 32],
    pub compile_request_hash:     [u8; 32],
    pub timestamp_unix:           u64,
    pub continuation_tail_bytes:  u32,           // length of build-specific tail in InferenceState
    pub semantic_schema_version:  u16,
    pub _reserved1:               [u8; 2],       // explicit trailing pad to 8-byte align
}
```

Pinned size 152 bytes, alignment 8 bytes; per-field offsets 0/4/8/40/72/104/136/144/148/150 follow from the field order. Layout invariants are enforced per the §4.2 strategy and gated by §3.1.4's `version::build_identity_layout` / `version::build_identity_offsets`.

Layout reasoning:

- `magic` first, four bytes, identifies the structure on inspection. A host-side dump tool can grep for `"GBLM"` in a ROM dump and find the block.
- `abi` second, three bytes, then a one-byte reserved pad to align the 32-byte hashes on 8-byte boundaries (good citizenship even though `[u8; 32]` has alignment 1).
- Four 32-byte hashes back-to-back: `build_hash` (encoded ROM core), `artifact_core_hash` (durable model identity), `runtime_nucleus_hash` (T2.5 drift gate), `compile_request_hash` (build inputs). All four are SHA-256 digest bytes in canonical digest order. `build_hash` must be computed over a canonical ROM image with the `build_hash` field itself zeroed out — otherwise the field would be self-referential. The exact hash domain is owned by F-B13/F-A5; F-A3 only requires the zeroed-field convention so host tools can reproduce the value.
- `timestamp_unix` for human-readable build identification, not for ordering. Reproducible builds set this field from `SOURCE_DATE_EPOCH`. If `SOURCE_DATE_EPOCH` is absent, the deterministic default is `0`; wall-clock timestamps are allowed only in explicitly non-reproducible developer builds.
- `continuation_tail_bytes`: the build-specific tail length so the host can size an `InferenceState` correctly.
- `semantic_schema_version`: the version of the `SemanticCheckpointSchema` contract the build adheres to.
- Trailing `_reserved1` pads the struct out to 152 bytes (an 8-byte multiple) so future revisions can extend without realigning. The reserved-byte test asserts `_reserved0 == 0` and `_reserved1 == [0, 0]`.

`BuildIdentityBlock` is `Copy` because it is POD-shaped: every field is `Copy`. The struct fields are public so layout assertions and offset_of! macros work, but invalid values are kept out of the type via the constructor + validator pair below. Host parsing uses a hand-written `from_bytes(&[u8; 152]) -> Result<Self, BuildIdentityError>` that validates the magic, decodes little-endian integers, and verifies reserved-byte zeroing. No API in `gbf-abi` returns a reference into an arbitrary byte buffer reinterpreted as a typed `#[repr(C)]` value.

```rust
impl BuildIdentityBlock {
    pub const MAGIC: [u8; 4] = *b"GBLM";

    /// Build a block with magic and reserved bytes set correctly. The only path to
    /// constructing a well-formed BuildIdentityBlock outside of explicit field-by-field
    /// initialization in tests.
    pub fn new(args: BuildIdentityArgs) -> Self {
        Self {
            magic: Self::MAGIC,
            abi: args.abi,
            _reserved0: 0,
            build_hash: args.build_hash,
            artifact_core_hash: args.artifact_core_hash,
            runtime_nucleus_hash: args.runtime_nucleus_hash,
            compile_request_hash: args.compile_request_hash,
            timestamp_unix: args.timestamp_unix,
            continuation_tail_bytes: args.continuation_tail_bytes,
            semantic_schema_version: args.semantic_schema_version,
            _reserved1: [0, 0],
        }
    }

    pub fn validate(&self) -> Result<(), BuildIdentityError>;
    pub fn from_bytes(bytes: &[u8; 152]) -> Result<Self, BuildIdentityError>;
    pub fn to_bytes(self) -> [u8; 152];
}

pub struct BuildIdentityArgs {
    pub abi: AbiVersion,
    pub build_hash: [u8; 32],
    pub artifact_core_hash: [u8; 32],
    pub runtime_nucleus_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub timestamp_unix: u64,
    pub continuation_tail_bytes: u32,
    pub semantic_schema_version: u16,
}
```

`validate` checks: magic equals `MAGIC`, both reserved bytes are zero, `abi.is_zero()` is false, and `semantic_schema_version >= 1`.

#### 3.1.4 Acceptance criteria (T-A3.1)

```bash
cargo check -p gbf-abi
cargo test -p gbf-abi -- version::current_constant_set            # CURRENT_ABI is non-zero
cargo test -p gbf-abi -- version::ord_total                       # PartialOrd matches lex order
cargo test -p gbf-abi -- version::semver_round_trip               # AbiVersion <-> SemVer in [0..=255]
cargo test -p gbf-abi --features host -- version::compatibility_envelope_validate
cargo test -p gbf-abi --features host -- version::compatibility_envelope_no_self
cargo test -p gbf-abi -- version::build_identity_layout                       # size_of == 152, asserted at compile time
cargo test -p gbf-abi -- version::build_identity_offsets                      # explicit offset_of! checks for every field
cargo test -p gbf-abi -- version::build_identity_constructor_sets_magic       # BuildIdentityBlock::new sets magic to b"GBLM"
cargo test -p gbf-abi -- version::build_identity_constructor_zeroes_reserved  # _reserved0 == 0 && _reserved1 == [0, 0]
cargo test -p gbf-abi -- version::build_identity_validate_rejects_bad_magic
cargo test -p gbf-abi -- version::build_identity_validate_rejects_nonzero_reserved
cargo test -p gbf-abi -- version::build_identity_from_bytes_round_trip        # to_bytes ∘ from_bytes == id over the 152-byte fixture
cargo test -p gbf-abi -- version::build_identity_serde_round_trip             # JSON round-trip
```

### 3.2 `continuation.rs` and `liveness.rs` — `InferenceState`, `LivenessCounters` (T-A3.2)

#### 3.2.1 `LivenessCounters`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LivenessCounters {
    pub progress_epoch:            u32,
    pub last_checkpoint:           CompactCheckpointId,  // u16
    pub no_progress_frames:        u16,
    pub livelock_threshold_frames: u16,
    pub _reserved:                 [u8; 2],              // align to 4
}
```

Pinned size 12 bytes, alignment 4 bytes; layout invariants per §4.2.

Methods:

```rust
impl LivenessCounters {
    pub fn new(threshold: u16) -> Self;
    pub fn record_progress(&mut self, cp: CompactCheckpointId);
    pub fn note_idle_frame(&mut self);
    pub fn is_livelocked(&self) -> bool;
    pub fn reset(&mut self, threshold: u16);
}
```

Semantics:

- `record_progress(cp)`: bump `progress_epoch` (saturating add, not wrap; once it saturates the runtime should checkpoint and reset); set `last_checkpoint = cp`; reset `no_progress_frames = 0`.
- `note_idle_frame()`: saturating-increment `no_progress_frames`. The cap is `u16::MAX`; once at the cap, additional calls are no-ops. The cap *itself* is not the livelock condition — `is_livelocked()` compares against `livelock_threshold_frames`.
- `is_livelocked()`: `no_progress_frames >= livelock_threshold_frames && livelock_threshold_frames != 0`. A zero threshold disables the check (used in `Bringup` profile).
- `FaultCode::RepeatedCheckpointNoProgress` is **not** derivable from `LivenessCounters` alone — there is no field here that records prior visits to a given checkpoint. The scheduler owns the per-checkpoint cache needed to detect "the same checkpoint was reached twice without intervening progress." `gbf-abi` exposes the typed fault code and the `last_checkpoint` field; F-A5 owns the detection algorithm. This split keeps the ABI a *state shape* and leaves *algorithms* to consumers.

`record_progress`'s `progress_epoch` increment is **saturating, not wrapping**. Wrapping would turn the monotonicity invariant into a lie at very long sessions; saturating preserves the invariant `progress_epoch_at_t1 >= progress_epoch_at_t0` for all t1 >= t0. The runtime's livelock detector has no need for an unbounded epoch because what matters is "did epoch advance since last frame", not the absolute value.

#### 3.2.2 `InferenceState`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InferenceStateHeader {
    pub abi:           AbiVersion,         // 3 bytes
    pub _reserved0:    u8,                 // complete first 4-byte word
    pub schema:        u16,                // versioned per-build state schema
    pub last_fault:    FaultCodeOptional,  // u16 sentinel-coded optional fault — fills the 2-byte gap after schema
    pub session_id:    u32,
    pub token_count:   u32,
    pub slice_id:      SliceId,            // u32 newtype
    pub liveness:      LivenessCounters,   // 12 bytes
}
```

Pinned size 32 bytes, alignment 4 bytes; per-field offsets `abi=0`, `_reserved0=3`, `schema=4`, `last_fault=6`, `session_id=8`, `token_count=12`, `slice_id=16`, `liveness=20`. Layout invariants per §4.2.

The full `InferenceState` value lives in WRAM and consists of an `InferenceStateHeader` followed by `BuildIdentityBlock::continuation_tail_bytes` of build-specific bytes. `gbf-abi` does not own a type for the tail; the build-specific sequence-state crate (`gbf-model` or its export) is the authority. F-A3 ships `InferenceStateHeader` and explicit helpers:

```rust
pub const fn header_size_bytes() -> usize { size_of::<InferenceStateHeader>() }

pub fn total_continuation_bytes(tail: u32) -> Result<usize, ContinuationError> {
    let tail_usize = usize::try_from(tail).map_err(|_| ContinuationError::TailTooLarge { tail })?;
    header_size_bytes()
        .checked_add(tail_usize)
        .ok_or(ContinuationError::TotalSizeOverflow { tail })
}

/// Decode an InferenceStateHeader from a byte buffer of length >= header_size_bytes().
/// Returns an owned, validated value — never a reference into the caller's buffer.
pub fn decode_header(buf: &[u8]) -> Result<InferenceStateHeader, ContinuationError>;

/// Decode the header and return the trailing slice (tail) as a borrow.
/// The caller asserts the expected tail length, which must match
/// `BuildIdentityBlock::continuation_tail_bytes`.
pub fn split_header_tail(
    buf: &[u8],
    expected_tail_bytes: u32,
) -> Result<(InferenceStateHeader, &[u8]), ContinuationError>;
```

`decode_header` and `split_header_tail` parse field-by-field via `from_le_bytes`, so they do not require `unsafe` and do not assume the input buffer's alignment matches `align_of::<InferenceStateHeader>()`.

#### 3.2.3 `FaultCodeOptional`

`FaultCode` itself is `#[repr(u16)]` with `FaultCode::None = 0` (§3.4). When stored in `InferenceState`, an `Option<FaultCode>` would not be `repr(C)`-portable. We use a transparent newtype:

```rust
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultCodeOptional(pub u16);

impl FaultCodeOptional {
    pub const NONE: Self = Self(0);
    pub fn from_option(code: Option<FaultCode>) -> Self;
    /// Decode the raw value into `Option<FaultCode>`. Returns `Err` for any non-zero raw
    /// value that is not a known FaultCode discriminant — the unknown raw value is
    /// preserved so callers can record it without coercion.
    pub fn decode(self) -> Result<Option<FaultCode>, UnknownFaultCode>;
    pub const fn raw(self) -> u16;
    pub fn is_none(self) -> bool;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownFaultCode {
    pub raw: u16,
}
```

A bare zero u16 always means "no fault" because `FaultCode::None = 0`. Any non-zero value is decoded against `FaultCode::ALL`. Unknown values are **not** silently coerced to `InternalAssertion`; they surface as `UnknownFaultCode { raw }` so the reader can preserve the exact offending discriminant for diagnostic capture and so a downstream reader can decide whether to treat it as a `HarnessProtocolError` or a forward-compatible warning.

#### 3.2.4 Acceptance criteria (T-A3.2)

```bash
cargo test -p gbf-abi -- continuation::header_layout                  # size_of == 32, offsets pinned
cargo test -p gbf-abi -- continuation::header_serde_round_trip
cargo test -p gbf-abi -- continuation::split_header_tail_validates_size
cargo test -p gbf-abi -- liveness::progress_advance                    # record_progress bumps epoch and resets idle frames
cargo test -p gbf-abi -- liveness::idle_frames_saturate                # note_idle_frame caps at u16::MAX
cargo test -p gbf-abi -- liveness::progress_epoch_saturates            # progress_epoch caps at u32::MAX
cargo test -p gbf-abi -- liveness::livelock_threshold_zero_disables    # threshold == 0 disables is_livelocked
cargo test -p gbf-abi -- liveness::livelock_threshold_fires_at_eq      # >= threshold, not > threshold
cargo test -p gbf-abi -- liveness::layout                              # size_of == 12, align_of == 4
```

### 3.3 `harness.rs` — control-plane blocks (T-A3.3)

#### 3.3.1 `HarnessOp` and `HarnessResultKind`

```rust
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessOp {
    Nop                = 0,
    StepSlice          = 1,
    RunUntilCheckpoint = 2,
    DumpArena          = 3,
    InjectFault        = 4,
    PowerCut           = 5,
    SetSession         = 6,
    GetState           = 7,
}

impl HarnessOp {
    pub const ALL: [HarnessOp; 8] = [
        Self::Nop, Self::StepSlice, Self::RunUntilCheckpoint, Self::DumpArena,
        Self::InjectFault, Self::PowerCut, Self::SetSession, Self::GetState,
    ];
    pub fn from_u16(raw: u16) -> Option<Self>;
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HarnessResultKind {
    Ok             = 0,
    Done           = 1,
    Fault          = 2,
    NotImplemented = 3,
    InvalidArgs    = 4,
}

impl HarnessResultKind {
    pub const ALL: [HarnessResultKind; 5] = [
        Self::Ok, Self::Done, Self::Fault, Self::NotImplemented, Self::InvalidArgs,
    ];
    pub fn from_u16(raw: u16) -> Option<Self>;
}
```

The `ALL` array test (`harness::op_kind_complete`) asserts `ALL.len() == 8` and that every enumerated discriminant appears exactly once. New variants must extend `ALL` or `cargo test` fails.

#### 3.3.2 Block layouts

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessCommandBlock {
    pub magic:    [u8; 4],   // b"HCMD"
    pub seq:      u32,
    pub op:       u16,        // HarnessOp discriminant; validated by from_u16
    pub doorbell: u8,         // 0 = clear, 1 = command ready
    pub _resv:    u8,         // must be zero
    pub args:     [u8; 32],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarnessResultBlock {
    pub magic:    [u8; 4],   // b"HRES"
    pub seq:      u32,        // matches command seq
    pub kind:     u16,        // HarnessResultKind discriminant
    pub ready:    u8,         // 0 = no result, 1 = result ready
    pub _resv:    u8,         // must be zero
    pub data:     [u8; 32],
}
```

Both blocks are pinned at size 44 bytes, alignment 4 bytes. Common per-field offsets: `magic=0`, `seq=4`, `op|kind=8`, `doorbell|ready=10`, `_resv=11`, `args|data=12`. Layout invariants per §4.2.

`op` and `kind` are *raw u16* in the layout, not the typed enum. The typed `HarnessOp::from_u16` and `HarnessResultKind::from_u16` handle decoding so unknown discriminants surface as `HarnessProtocolError` rather than as undefined behavior. (A `#[repr(u16)]` enum with an unknown discriminant in memory is technically UB if read directly; routing through `from_u16` keeps the type system honest.)

Constructors stamp the magic and zero the reserved byte; the validators check both:

```rust
impl HarnessCommandBlock {
    pub const MAGIC: [u8; 4] = *b"HCMD";

    pub fn new(seq: u32, op: HarnessOp, args: [u8; 32]) -> Self;
    pub fn decode_op(&self) -> Result<HarnessOp, HarnessProtocolError>;
    pub fn validate(&self) -> Result<(), HarnessProtocolError>;
}

impl HarnessResultBlock {
    pub const MAGIC: [u8; 4] = *b"HRES";

    pub fn new(seq: u32, kind: HarnessResultKind, data: [u8; 32]) -> Self;
    pub fn decode_kind(&self) -> Result<HarnessResultKind, HarnessProtocolError>;
    pub fn validate_for_command(&self, command_seq: u32) -> Result<(), HarnessProtocolError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessProtocolError {
    BadMagic { observed: [u8; 4], expected: [u8; 4] },
    NonZeroReserved { value: u8 },
    UnknownOp { raw: u16 },
    UnknownResultKind { raw: u16 },
    SeqMismatch { command: u32, result: u32 },
}
```

The block magic constants stamp at construction; reserved bytes are always zero in `new`-built blocks; `validate*` rejects any non-zero `_resv` value or magic mismatch on parsed blocks.

#### 3.3.3 Doorbell protocol

The dedicated `doorbell` field on `HarnessCommandBlock` is the command-doorbell; the `ready` field on `HarnessResultBlock` is the result-ready signal. The full 32-byte `args` and `data` payloads are reserved entirely for op-specific arguments and results — payload capacity is **not** reduced by doorbell stealing. The host writes the full command (`magic`, `seq`, `op`, `args`), then sets `doorbell = 1` last. The ROM polls `doorbell` at safe points; on observing 1 it copies the block, executes, writes the result block (`magic`, `seq`, `kind`, `data`), sets `ready = 1`, then clears `doorbell = 0`. The host ignores any result whose `seq` does not match the command it issued, and clears the previous result's `ready` byte to 0 before issuing a new command. F-D2 harness control plane owns the polling and serialization. F-A3 ships:

```rust
pub mod doorbell {
    pub const COMMAND_DOORBELL_OFFSET: usize = 10;   // HarnessCommandBlock::doorbell
    pub const RESULT_READY_OFFSET:     usize = 10;   // HarnessResultBlock::ready
    pub const DOORBELL_RAISED: u8 = 1;
    pub const DOORBELL_CLEAR:  u8 = 0;
}
```

#### 3.3.4 Payload conventions

All multi-byte payload fields are little-endian. Payload bytes not used by an operation must be zeroed by the sender and ignored by the receiver.

| Op | Command `args` | Result `data` |
|----|----------------|---------------|
| `Nop`                | all zero | all zero |
| `StepSlice`          | `args[0..4] = max_m_cycles: u32` | `data[0..4] = executed_m_cycles: u32` |
| `RunUntilCheckpoint` | `args[0..2] = target: CompactCheckpointId`, `args[2..6] = max_m_cycles: u32` | `data[0..2] = reached: CompactCheckpointId`, `data[2..4] = fault_code: u16` |
| `DumpArena`          | `args[0..2] = address: u16`, `args[2..4] = len: u16` | descriptor only; bulk bytes are read through the harness memory window |
| `InjectFault`        | `args[0..2] = fault_code: u16` | `data[0..2] = accepted_fault_code: u16` |
| `PowerCut`           | `args[0] = power_cut_mode: u8` | all zero |
| `SetSession`         | `args[0..4] = session_id: u32` | `data[0..4] = session_id: u32` |
| `GetState`           | all zero | `data[0..32] = InferenceStateHeader` |

`GetState` returns the entire 32-byte `InferenceStateHeader` because the doorbell is now in its own field. F-D2 may add additional ops in reserved discriminant ranges (see §3.3.5).

#### 3.3.5 Acceptance criteria (T-A3.3)

```bash
cargo test -p gbf-abi -- harness::layout                              # size 44 for both blocks; offset pinned
cargo test -p gbf-abi -- harness::constructor_sets_magic              # HarnessCommandBlock::new + HarnessResultBlock::new
cargo test -p gbf-abi -- harness::constructor_zeroes_reserved
cargo test -p gbf-abi -- harness::validate_rejects_bad_magic
cargo test -p gbf-abi -- harness::validate_rejects_nonzero_reserved
cargo test -p gbf-abi -- harness::op_kind_complete                    # ALL covers all variants, no duplicates
cargo test -p gbf-abi -- harness::op_from_u16_rejects_unknown          # unknown discriminant => Err(UnknownOp)
cargo test -p gbf-abi -- harness::result_kind_from_u16_rejects_unknown
cargo test -p gbf-abi -- harness::seq_mismatch_rejected
cargo test -p gbf-abi -- harness::serde_round_trip
cargo test -p gbf-abi -- harness::doorbell_constants_distinct          # DOORBELL_RAISED != DOORBELL_CLEAR
```

### 3.4 `fault.rs` — `FaultCode`, `FaultDomain`, `FaultSnapshot`, `FaultPolicy` (T-A3.4)

#### 3.4.1 `FaultCode`

```rust
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FaultCode {
    None                          = 0x0000,
    AbiVersionMismatch            = 0x0001,
    BuildIdentityMismatch         = 0x0002,
    PersistChecksum               = 0x0010,
    PersistTornWrite              = 0x0011,
    PersistSchemaUnknown          = 0x0012,
    BankShadowDivergence          = 0x0020,
    UnauthorizedMbcWrite          = 0x0021,
    LeaseUnbalanced               = 0x0022,
    YieldTooLong                  = 0x0030,
    InterruptLatencyExceeded      = 0x0031,
    LivenessTimeout               = 0x0032,
    RepeatedCheckpointNoProgress  = 0x0033,
    UiCommitOutsideLegalMode      = 0x0040,
    UnknownChecksumKind           = 0x0050,
    UnknownPersistKind            = 0x0051,
    HarnessProtocolError          = 0x0060,
    TraceBudgetExceeded           = 0x0070,
    CalibrationDrift              = 0x0080,
    InternalAssertion             = 0xFF00,
}

impl FaultCode {
    pub const ALL: &'static [FaultCode] = &[ /* every variant exactly once */ ];
    pub fn from_u16(raw: u16) -> Option<Self>;
}
```

Discriminant ranges are documented:

| Range            | Domain        |
|------------------|---------------|
| `0x0000`         | None / OK sentinel |
| `0x0001..=0x000F`| Boot (ABI / build identity) |
| `0x0010..=0x001F`| Persistence   |
| `0x0020..=0x002F`| Banking       |
| `0x0030..=0x0031`| Scheduling (yield / interrupt latency) |
| `0x0032..=0x003F`| Liveness      |
| `0x0040..=0x004F`| UI            |
| `0x0050..=0x005F`| Schema / unknown-kind |
| `0x0060..=0x006F`| Harness       |
| `0x0070..=0x007F`| Trace         |
| `0x0080..=0x008F`| Calibration / drift |
| `0xFF00..=0xFFFF`| Internal assertion |

A range gap inside a documented domain is reserved for future expansion. Test `fault::range_partition` asserts that every member of `FaultCode::ALL` falls inside its declared domain range.

#### 3.4.2 `FaultDomain`

```rust
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FaultDomain {
    None        = 0x00,
    Boot        = 0x01,
    Persistence = 0x02,
    Banking     = 0x03,
    Scheduling  = 0x04,
    Liveness    = 0x05,
    Ui          = 0x06,
    Schema      = 0x07,
    Harness     = 0x08,
    Trace       = 0x09,
    Calibration = 0x0A,
    Internal    = 0xFF,
}

impl FaultDomain {
    pub const ALL: &'static [FaultDomain] = &[ /* every variant exactly once */ ];
    pub fn from_u16(raw: u16) -> Option<Self>;
    /// The discriminant range covered by this domain inside FaultCode (see §3.4.1 table).
    pub const fn range(self) -> core::ops::RangeInclusive<u16>;
}

pub fn classify_fault(code: FaultCode) -> FaultDomain;
```

`classify_fault` is total: every `FaultCode::ALL` entry has a `FaultDomain` mapping, asserted by `fault::code_to_domain_total`.

Required mappings:

```text
FaultCode::None                         => FaultDomain::None
FaultCode::AbiVersionMismatch           => FaultDomain::Boot
FaultCode::BuildIdentityMismatch        => FaultDomain::Boot
FaultCode::PersistChecksum              => FaultDomain::Persistence
FaultCode::PersistTornWrite             => FaultDomain::Persistence
FaultCode::PersistSchemaUnknown         => FaultDomain::Persistence
FaultCode::BankShadowDivergence         => FaultDomain::Banking
FaultCode::UnauthorizedMbcWrite         => FaultDomain::Banking
FaultCode::LeaseUnbalanced              => FaultDomain::Banking
FaultCode::YieldTooLong                 => FaultDomain::Scheduling
FaultCode::InterruptLatencyExceeded     => FaultDomain::Scheduling
FaultCode::LivenessTimeout              => FaultDomain::Liveness
FaultCode::RepeatedCheckpointNoProgress => FaultDomain::Liveness
FaultCode::UiCommitOutsideLegalMode     => FaultDomain::Ui
FaultCode::UnknownChecksumKind          => FaultDomain::Schema
FaultCode::UnknownPersistKind           => FaultDomain::Schema
FaultCode::HarnessProtocolError         => FaultDomain::Harness
FaultCode::TraceBudgetExceeded          => FaultDomain::Trace
FaultCode::CalibrationDrift             => FaultDomain::Calibration
FaultCode::InternalAssertion            => FaultDomain::Internal
```

The `range_partition` test additionally asserts `classify_fault(c).range().contains(&(c as u16))` for every code, so the per-domain range table and the per-code mapping cannot drift apart.

#### 3.4.3 `FaultSnapshot`

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterSnapshot {
    pub a:  u8, pub f:  u8,
    pub b:  u8, pub c:  u8,
    pub d:  u8, pub e:  u8,
    pub h:  u8, pub l:  u8,
    pub sp: u16,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultSnapshot {
    pub code:           u16,         // FaultCode discriminant
    pub domain:         u16,         // FaultDomain ordinal
    pub at_pc:          u16,
    pub at_bank:        u16,         // RomBank, widened from u8 to align
    pub at_checkpoint:  CompactCheckpointId, // u16
    pub _resv:          u16,         // pad to 4
    pub regs:           RegisterSnapshot,
    pub _resv1:         [u8; 2],     // realign to 4
    pub liveness:       LivenessCounters, // 12 bytes
}
```

Pinned sizes: `RegisterSnapshot` 10 bytes, `FaultSnapshot` 36 bytes. Layout invariants per §4.2.

Fields are widened to natural u16/u32 boundaries so layout is portable across compilers. The `domain` field stores the **pinned `FaultDomain` discriminant** (per §3.4.2's `#[repr(u16)]` table), not an array ordinal. Host parsing decodes via `FaultDomain::from_u16`. For known fault codes, `FaultSnapshot::validate()` asserts:

```rust
snapshot.domain_decoded()? == classify_fault(snapshot.code_decoded()?)
```

Unknown raw fault codes or domains are reported as decode errors (`SnapshotDecodeError::UnknownFaultCode { raw }` / `UnknownFaultDomain { raw }`) while preserving the raw `code` and `domain` u16 values inside the error so an emulator dump still captures the offending value verbatim.

#### 3.4.4 `FaultPolicy` and `RecoveryAction`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    ColdStart,
    DemoteToSafeMode,
    AbortAndPanic,
    BootValidationOnly,
    RetrySlice,
    DropTrace,
    HardReset,
}

impl RecoveryAction {
    pub const ALL: &'static [RecoveryAction] = &[ /* 7 variants */ ];
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaultPolicy {
    pub by_domain: alloc::collections::BTreeMap<FaultDomain, RecoveryAction>,
    pub default_action: RecoveryAction,
}

#[cfg(feature = "host")]
impl FaultPolicy {
    pub fn action_for(&self, code: FaultCode) -> RecoveryAction;
    pub fn validate(&self) -> Result<(), FaultPolicyError>;
}
```

`FaultPolicy::default_action` must not be `BootValidationOnly` (that is a boot-only legal action). `validate` enforces this.

#### 3.4.5 `BootValidationPlan`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootValidationPlan {
    pub validate_persistence:           bool,
    pub validate_runtime_nucleus_hash:  bool,
    pub validate_artifact_core_hash:    bool,
    pub validate_compile_request_hash:  bool,
    pub persist_scan_policy:            PersistScanPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PersistScanPolicy {
    StrictCriticalOnly,
    ScanAll,
}
```

#### 3.4.6 Acceptance criteria (T-A3.4)

```bash
cargo test -p gbf-abi -- fault::all_unique_discriminants     # ALL has no duplicates
cargo test -p gbf-abi -- fault::code_from_u16_round_trip
cargo test -p gbf-abi -- fault::code_to_domain_total          # classify_fault total over ALL
cargo test -p gbf-abi -- fault::range_partition               # every code falls in its domain range
cargo test -p gbf-abi -- fault::register_snapshot_layout      # size_of == 10
cargo test -p gbf-abi -- fault::snapshot_layout               # size_of == 36, offsets pinned
cargo test -p gbf-abi -- fault::snapshot_domain_matches_code  # snapshot.domain == classify_fault(snapshot.code)
cargo test -p gbf-abi -- fault::recovery_action_exhaustive
cargo test -p gbf-abi --features host -- fault::policy_default_action_validation
cargo test -p gbf-abi --features host -- fault::policy_action_for_falls_back_to_default
```

### 3.5 `interrupt.rs` — `InterruptPolicy`, `ResourceLease`, `ResourceLeaseKind` (T-A3.5)

#### 3.5.1 `InterruptPolicy`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InterruptPolicy {
    Enabled,
    ShortCriticalSection,
    Disabled,
}

impl InterruptPolicy {
    pub const ALL: [InterruptPolicy; 3] = [
        Self::Enabled, Self::ShortCriticalSection, Self::Disabled,
    ];
}
```

Documented invariants:

- `Disabled` is legal only at boot. The runtime-side validator (F-A5) rejects post-boot slices with `InterruptPolicy::Disabled`.
- `ShortCriticalSection` declares an EI/DI window of approximately 10 M-cycles. The exact bound lives in F-A4; F-A3 only declares the policy variant.

#### 3.5.2 IDs

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LeaseId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SliceId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OverlayId(pub u16);
```

These IDs are newtypes over the underlying primitive — same wire size, but the type system distinguishes them. An overflow at `u32::MAX` is a build-pipeline bug; F-A3 documents the bound and asserts it via `LeaseId::ALL_VALUES_FIT` in tests.

#### 3.5.3 Bindings

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RomWindowBinding {
    pub bank: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SramPageBinding {
    pub page:    u8,
    pub enabled: bool,
}
```

`RomWindowBinding::bank` is `u16` because MBC5 has 9-bit bank registers (max 0x01FF). It is *not* the file-offset bank; it is the runtime CPU view.

#### 3.5.4 `ResourceLeaseKind` and `ResourceLease`

`ResourceLeaseKind` and `ResourceLease` are typed validation vocabulary, **not** ROM-resident byte layouts. They are serialized through serde in host/runtime reports and consumed by `ResourceStateValidation` (F-B11). They are intentionally Rust enums with non-`repr(C)` data variants and **must not** be read from cartridge memory by casting bytes to Rust enums. If F-A4/F-B11 later needs a byte-stable lease log, that feature must introduce a separate `#[repr(C)] ResourceLeaseRecord` with a raw `kind_tag: u16` and a fixed-size payload — that wire form is out of scope for F-A3.

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceLeaseKind {
    RomWindow(RomWindowBinding),
    SramPage(SramPageBinding),
    Overlay(OverlayId),
    InterruptMask(InterruptPolicy),
}

impl ResourceLeaseKind {
    pub fn yield_safe(&self) -> bool {
        match self {
            Self::RomWindow(_)        => false, // illegal to yield while holding switchable bank
            Self::SramPage(_)         => true,  // runtime saves/restores SRAM bank state
            Self::Overlay(_)          => false, // overlay regions cannot be reclaimed mid-yield
            Self::InterruptMask(p)    => matches!(p, InterruptPolicy::Enabled),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLease {
    pub id:           LeaseId,
    pub kind:         ResourceLeaseKind,
    pub acquired_in:  SliceId,
    pub released_in:  Option<SliceId>,   // None means the lease is still active
}

impl ResourceLease {
    pub fn yield_safe(&self) -> bool { self.kind.yield_safe() }
    pub fn is_active(&self) -> bool { self.released_in.is_none() }
    pub fn is_balanced(&self) -> bool {
        self.released_in
            .map(|released| released.0 >= self.acquired_in.0)
            .unwrap_or(false)
    }
}
```

`released_in: Option<SliceId>` removes the previous-encoding ambiguity where `released_in == acquired_in` could have meant either "released in the same slice" or "not yet released". Active leases now use `None` explicitly.

The `yield_safe` table is a hard invariant for `ResourceStateValidation` (F-B11). A test (`interrupt::lease_yield_safety_table`) pins every variant's expected boolean explicitly so a future refactor cannot silently flip one.

#### 3.5.5 Acceptance criteria (T-A3.5)

```bash
cargo test -p gbf-abi -- interrupt::policy_enum_complete
cargo test -p gbf-abi -- interrupt::lease_yield_safety_table
cargo test -p gbf-abi -- interrupt::lease_id_distinct_from_slice_id  # compile-fail test via trybuild (or omit; the Rust type checker enforces it natively)
cargo test -p gbf-abi -- interrupt::serde_round_trip                  # all variants
cargo test -p gbf-abi -- interrupt::balanced_predicate                # is_balanced returns false for active leases
cargo test -p gbf-abi -- interrupt::active_predicate                  # is_active iff released_in is None
cargo test -p gbf-abi -- interrupt::sram_page_disabled_state          # enabled: false is legal
```

### 3.6 `checkpoint.rs` — `SemanticCheckpointId`, `CompactCheckpointId`, `SemanticCheckpointSchema` (T-A3.6)

#### 3.6.1 IDs

```rust
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SemanticCheckpointId(alloc::borrow::Cow<'static, str>);

#[cfg(feature = "alloc")]
impl SemanticCheckpointId {
    pub fn from_static(s: &'static str) -> Result<Self, CheckpointIdError>;
    pub fn from_owned(s: alloc::string::String) -> Result<Self, CheckpointIdError>;
    pub fn as_str(&self) -> &str;
}

#[cfg(feature = "alloc")]
impl serde::Serialize for SemanticCheckpointId {
    /* serialize as the borrowed &str */
}

#[cfg(feature = "alloc")]
impl<'de> serde::Deserialize<'de> for SemanticCheckpointId {
    /* deserialize as owned String, then validate via from_owned; reject on parse error */
}

#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct CompactCheckpointId(pub u16);

impl CompactCheckpointId {
    pub const NONE: Self = Self(0);
}
```

Validation rules for `SemanticCheckpointId::from_*`:

- Non-empty.
- Dot-separated segments; each segment matches `[a-z0-9_]+`.
- No leading or trailing dot, no consecutive dots.
- Length ≤ 128 bytes.
- Reserved: empty string, single `.`, `..`.

`CompactCheckpointId::NONE = 0` is the sentinel for "no checkpoint reached yet" used inside `LivenessCounters`.

#### 3.6.2 `SemanticStratum`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum SemanticStratum {
    Denotation,
    Artifact,
    Operational,
}

impl SemanticStratum {
    pub const ALL: [SemanticStratum; 3] = [Self::Denotation, Self::Artifact, Self::Operational];
}
```

The three strata mirror planv0.md's three-oracle decomposition:

- `Denotation` — `DenotationalOracle` checkpoints (target-independent reference meaning).
- `Artifact` — `ArtifactOracle` checkpoints (canonical logical form, no tiling/banking).
- `Operational` — `ScheduleOracle`/runtime checkpoints (scheduled execution).

#### 3.6.3 `SemanticCheckpointSchema`

```rust
#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticCheckpointSchema {
    pub schema_version:       u16,
    pub abi_version:          AbiVersion,
    pub build_hash:           [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub checkpoints:          alloc::vec::Vec<CheckpointEntry>,
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointEntry {
    pub semantic:    SemanticCheckpointId,
    pub compact:     CompactCheckpointId,
    pub stratum:     SemanticStratum,
    pub source_op:   Option<alloc::borrow::Cow<'static, str>>,
}

#[cfg(feature = "host")]
impl SemanticCheckpointSchema {
    pub fn validate(&self) -> Result<(), SchemaValidationError>;
    pub fn resolve_compact(&self, id: CompactCheckpointId) -> Option<&SemanticCheckpointId>;
    pub fn resolve_semantic(&self, id: &SemanticCheckpointId) -> Option<CompactCheckpointId>;
    pub fn group_by_stratum(&self) -> alloc::collections::BTreeMap<SemanticStratum, alloc::vec::Vec<&CheckpointEntry>>;
}
```

Validation rules:

- The schema's `build_hash` and `compile_request_hash` must match the attached `BuildIdentityBlock`. Pairing a runtime to its sidecar by `schema_version` alone is **not** enough, because `CompactCheckpointId`s are explicitly build-local — two builds with the same schema version but different code can mint conflicting compact ids.
- No two entries share a `compact` id.
- No two entries share a `semantic` id.
- `CompactCheckpointId(0)` (i.e. `CompactCheckpointId::NONE`) is reserved and may not appear.
- Every `semantic` id passes `SemanticCheckpointId::from_*` validation (so deserialize cannot smuggle in invalid IDs).

#### 3.6.4 Acceptance criteria (T-A3.6)

```bash
cargo test -p gbf-abi -- checkpoint::semantic_id_validation_basic       # accepts "layer.3.router.post_top1"
cargo test -p gbf-abi -- checkpoint::semantic_id_rejects_uppercase
cargo test -p gbf-abi -- checkpoint::semantic_id_rejects_double_dot
cargo test -p gbf-abi -- checkpoint::semantic_id_rejects_leading_dot
cargo test -p gbf-abi -- checkpoint::compact_none_sentinel
cargo test -p gbf-abi -- checkpoint::stratum_exhaustive
cargo test -p gbf-abi --features host -- checkpoint::schema_validates_unique_compact
cargo test -p gbf-abi --features host -- checkpoint::schema_validates_unique_semantic
cargo test -p gbf-abi --features host -- checkpoint::schema_rejects_compact_zero
cargo test -p gbf-abi --features host -- checkpoint::schema_resolve_round_trip
cargo test -p gbf-abi --features host -- checkpoint::serde_round_trip
cargo test -p gbf-abi --features host -- checkpoint::group_by_stratum_partitions
```

### 3.7 `trace.rs` — `TraceEvent`, `TraceProbeId`, `TraceBudget` (T-A3.7)

#### 3.7.1 Probe types

```rust
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct TraceProbeId(pub u16);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeLevel {
    Always,
    OnError,
    Verbose,
}

impl ProbeLevel {
    pub const ALL: [ProbeLevel; 3] = [Self::Always, Self::OnError, Self::Verbose];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProbeBudgetClass {
    PerSlice,
    PerFrame,
    PerSession,
}

impl ProbeBudgetClass {
    pub const ALL: [ProbeBudgetClass; 3] = [Self::PerSlice, Self::PerFrame, Self::PerSession];
}
```

#### 3.7.2 `TraceEvent`

The canonical layout orders the 32-bit fields first, then the 16-bit identifiers, so the compiler has no implicit padding to insert and the struct fits exactly into a 32-byte slot:

```rust
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceEvent {
    pub seq:                u32,                 // 0..4
    pub timestamp_m_cycles: u32,                 // 4..8
    pub slice:              SliceId,             // 8..12
    pub probe:              TraceProbeId,        // 12..14
    pub checkpoint:         CompactCheckpointId, // 14..16
    pub data:               [u8; 16],            // 16..32
}
```

Pinned size 32 bytes; the per-field byte ranges above are the canonical offsets. Layout invariants per §4.2.

A 32-byte event divides cleanly into a 1 KiB trace page (32 events) which matches the SRAM page size assumption used in F-D3 / planv0.md.

`seq` and `timestamp_m_cycles` are modulo-`u32` counters. Consumers must order events primarily by ring position and `seq` equality/adjacency, not by treating `timestamp_m_cycles` as an unbounded wall-clock value.

#### 3.7.3 `TraceBudget`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceBudget {
    pub max_events_per_slice: u16,
    pub max_bytes_per_frame:  u16,
    pub drop_policy:          TraceDropPolicy,
}

impl TraceBudget {
    pub fn new(
        max_events_per_slice: u16,
        max_bytes_per_frame: u16,
        drop_policy: TraceDropPolicy,
    ) -> Result<Self, TraceBudgetError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceBudgetError {
    ZeroEventsWithNonzeroBytes,
    NonzeroEventsWithZeroBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraceDropPolicy {
    DropOldest,
    DropNewest,
    HaltAndFault,
}

impl TraceDropPolicy {
    pub const ALL: [TraceDropPolicy; 3] = [
        Self::DropOldest, Self::DropNewest, Self::HaltAndFault,
    ];
}
```

`HaltAndFault` raises `FaultCode::TraceBudgetExceeded`. Used in invariant-mode (`Recovery` profile) builds. The default policy under planv0.md "live debugging" assumption is `DropOldest`.

#### 3.7.4 `TraceProbeRegistry` (host)

To prevent drift between probe identity and probe metadata, F-A3 ships a minimal host-only registry. It pairs with a build via `build_hash` so a registry sidecar cannot be misapplied to a different build:

```rust
#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceProbeRegistry {
    pub abi_version: AbiVersion,
    pub build_hash:  [u8; 32],
    pub probes:      alloc::vec::Vec<TraceProbeEntry>,
}

#[cfg(feature = "host")]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceProbeEntry {
    pub probe:              TraceProbeId,
    pub level:              ProbeLevel,
    pub budget_class:       ProbeBudgetClass,
    pub payload_schema_tag: alloc::borrow::Cow<'static, str>,
}
```

Validation rules:

- `build_hash` must match the attached `BuildIdentityBlock`.
- No two entries share a `TraceProbeId`.
- `payload_schema_tag` is a free-form identifier; F-A3 does not own its semantics (F-D3 does).

#### 3.7.5 Acceptance criteria (T-A3.7)

```bash
cargo test -p gbf-abi -- trace::event_layout                # size_of == 32, offsets pinned
cargo test -p gbf-abi -- trace::probe_level_exhaustive
cargo test -p gbf-abi -- trace::probe_budget_class_exhaustive
cargo test -p gbf-abi -- trace::drop_policy_exhaustive
cargo test -p gbf-abi -- trace::serde_round_trip
cargo test -p gbf-abi -- trace::trace_budget_constructor_rejects_inconsistent
cargo test -p gbf-abi -- trace::trace_budget_constructor_accepts_zero_zero
cargo test -p gbf-abi -- trace::seq_modulo_documented   # consumers must not assume unbounded seq/timestamp
```

### 3.8 `lib.rs` — re-exports, feature gates, `unsafe`-free assertion

```rust
//! Live execution ABI shared by compiler, runtime, harnesses, and emulator adapters.
//!
//! See `history/rfcs/F-A3-gbf-abi.md` for design rationale.

#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]

#[cfg(any(feature = "alloc", feature = "host"))]
extern crate alloc;

pub mod checkpoint;
pub mod continuation;
pub mod fault;
pub mod harness;
pub mod interrupt;
pub mod liveness;
pub mod trace;
pub mod version;

pub use checkpoint::{CompactCheckpointId, SemanticStratum};
#[cfg(feature = "alloc")]
pub use checkpoint::SemanticCheckpointId;
pub use continuation::{InferenceStateHeader, FaultCodeOptional};
pub use fault::{FaultCode, FaultDomain, FaultSnapshot, RegisterSnapshot, RecoveryAction, BootValidationPlan, PersistScanPolicy};
pub use harness::{HarnessCommandBlock, HarnessResultBlock, HarnessOp, HarnessResultKind};
pub use interrupt::{InterruptPolicy, ResourceLease, ResourceLeaseKind, RomWindowBinding, SramPageBinding, LeaseId, SliceId, OverlayId};
pub use liveness::LivenessCounters;
pub use trace::{TraceEvent, TraceProbeId, ProbeLevel, ProbeBudgetClass, TraceBudget, TraceDropPolicy};
pub use version::{AbiVersion, BuildIdentityBlock, CURRENT_ABI};

#[cfg(feature = "host")]
pub use checkpoint::{CheckpointEntry, SemanticCheckpointSchema, SchemaValidationError};
#[cfg(feature = "host")]
pub use fault::{FaultPolicy, FaultPolicyError};
#[cfg(feature = "host")]
pub use version::{CompatibilityEnvelope, CompatibilityError};
```

The `#![forbid(unsafe_code)]` lint denies any `unsafe` token in the crate. The crate intentionally does *not* use `bytemuck::Pod` / `Zeroable` derives, because those macros expand to `unsafe impl` declarations which are incompatible with the lint. All byte-level parsing goes through explicit `from_le_bytes` helpers.

## 4. Cross-cutting concerns

### 4.1 `no_std + alloc`, `host` feature

The crate compiles in three configurations:

| Configuration | Cargo features | Available types |
|---------------|----------------|-----------------|
| Bare runtime  | (none)         | All `#[repr(C)]` POD types — version, fault, harness, interrupt, trace, liveness, continuation, checkpoint IDs |
| `+alloc`      | `alloc`        | Above + `SemanticCheckpointId` (uses `Cow<'static, str>` from `alloc`) |
| `+host`       | `alloc, host`  | Above + `CompatibilityEnvelope`, `SemanticCheckpointSchema`, `FaultPolicy` |

`std` is enabled transitively via `host` for serde-json round-trip tests; the runtime build uses `default-features = false`.

### 4.2 Layout assertion strategy

Every `#[repr(C)]` struct gets:

1. `const_assert_eq!(size_of::<T>(), <expected>)`.
2. `const_assert_eq!(align_of::<T>(), <expected>)`.
3. One `const_assert_eq!(memoffset::offset_of!(T, f), <offset>)` per field.
4. A reserved-bytes test asserting `_resv` fields default-zero.
5. A serde round-trip test (host feature on).
6. A magic-bytes test where applicable (`BuildIdentityBlock`, `HarnessCommandBlock`, `HarnessResultBlock`).

`memoffset` is a build-dependency-only crate; it produces compile-time offsets via a stable macro. `static_assertions` provides `const_assert_eq!`.

The size constants are pinned per type:

| Type                    | Size (bytes) | Align (bytes) | ROM-resident? |
|-------------------------|-------------:|--------------:|---------------|
| `AbiVersion`            |            3 |             1 | yes (inside BuildIdentityBlock) |
| `BuildIdentityBlock`    |          152 |             8 | yes |
| `LivenessCounters`      |           12 |             4 | yes (inside InferenceStateHeader) |
| `InferenceStateHeader`  |           32 |             4 | yes (WRAM-resident) |
| `FaultCodeOptional`     |            2 |             2 | yes (inside InferenceStateHeader) |
| `RegisterSnapshot`      |           10 |             2 | yes (inside FaultSnapshot) |
| `FaultSnapshot`         |           36 |             4 | yes (SRAM-resident on fault) |
| `HarnessCommandBlock`   |           44 |             4 | yes (SRAM-resident) |
| `HarnessResultBlock`    |           44 |             4 | yes (SRAM-resident) |
| `TraceEvent`            |           32 |             4 | yes (SRAM trace page) |

Total ROM/RAM-resident ABI footprint is small (≤ 400 bytes for one of each), but every byte is pinned.

### 4.3 Endianness

Game Boy is little-endian, and all host platforms we currently target (x86_64, aarch64) are little-endian. F-A3 nevertheless documents the expectation: every multi-byte integer in a `#[repr(C)]` type stored in cartridge memory is **little-endian**. Host-side parsers (F-H3, F-D2, F-D3) must use `u16::from_le_bytes` / `u32::from_le_bytes` / `u64::from_le_bytes` rather than native reads. F-A3 does not provide `unsafe` transmute helpers and does not depend on `bytemuck`; the explicit `from_le_bytes` helpers are the only blessed byte path inside the crate.

### 4.4 `Drop` and `Copy`

Every ROM-resident type implements `Copy + Clone + Sized` and does not implement `Drop`. A test (`assert_pod_no_drop`) uses `static_assertions::assert_not_impl_all!` to fail compilation if a `Drop` is added.

### 4.5 Adapter shape for downstream consumers

F-C1/F-C2/F-C3 (oracle stack) consume `SemanticCheckpointId` and `CompactCheckpointId` via:

```rust
pub trait CheckpointResolver {
    fn resolve(&self, semantic: &SemanticCheckpointId) -> Option<CompactCheckpointId>;
    fn resolve_back(&self, compact: CompactCheckpointId) -> Option<&SemanticCheckpointId>;
    fn stratum(&self, compact: CompactCheckpointId) -> Option<SemanticStratum>;
}
```

`SemanticCheckpointSchema` implements `CheckpointResolver` under `feature = "host"`. Consumers depend on the trait so they can swap a runtime-side compact-only resolver during tests.

F-A4 (`bd-1sv`) consumes `ResourceLeaseKind`, `ResourceLease`, `LeaseId`, `SliceId`, `RomWindowBinding`, `SramPageBinding`. F-A4's `BankLease` builders construct `ResourceLease` records that flow into F-B11's `ResourceStateValidation`.

F-D5 (`bd-3ot1`) consumes `FaultPolicy` (host feature). F-D5 owns the policy *content*; F-A3 owns the *types*.

## 5. Errors

Every fallible API in `gbf-abi` returns a typed error:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckpointIdError {
    Empty,
    TooLong { len: usize, max: usize },
    InvalidChar { byte: u8, position: usize },
    LeadingDot,
    TrailingDot,
    DoubleDot { position: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaValidationError {
    DuplicateCompact { compact: CompactCheckpointId },
    DuplicateSemantic { semantic: SemanticCheckpointId },
    ReservedCompactZero,
    InvalidSemanticId { semantic: alloc::string::String, error: CheckpointIdError },
    BuildHashMismatch { expected: [u8; 32], observed: [u8; 32] },
    CompileRequestHashMismatch { expected: [u8; 32], observed: [u8; 32] },
    AbiVersionMismatch { expected: AbiVersion, observed: AbiVersion },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbiVersionError {
    Zero,
    Unsupported { observed: AbiVersion, current: AbiVersion },
    SemVerOutOfRange { major: u64, minor: u64, patch: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildIdentityError {
    BadMagic { observed: [u8; 4], expected: [u8; 4] },
    BadAbi { error: AbiVersionError },
    Truncated { expected: usize, observed: usize },
    NonZeroReserved { offset: usize, value: u8 },
    BadSchemaVersion { observed: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultPolicyError {
    DefaultActionIsBootValidationOnly,
    UnknownDomain { ordinal: u16 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompatibilityError {
    DuplicatesCurrent { duplicate: AbiVersion },
    BackwardLargerThanCurrent { offender: AbiVersion },
    ForwardSmallerThanCurrent { offender: AbiVersion },
    DuplicateInBackward { offender: AbiVersion },
    DuplicateInForward { offender: AbiVersion },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationError {
    HeaderTruncated { observed: usize },
    TailTruncated { expected_tail: u32, observed: usize },
    TailTooLarge { tail: u32 },
    TotalSizeOverflow { tail: u32 },
    BadAbi { observed: AbiVersion, expected: AbiVersion },
    UnknownFaultCodeInLastFault { raw: u16 },
    BadSchemaVersion { observed: u16, expected: u16 },
}
```

Every error type implements `core::fmt::Display`. Under `feature = "std"`, every error type also implements `std::error::Error`. The `Display` implementations write directly to the formatter and do not allocate.

## 6. Testing strategy

### 6.1 Layers

- **Unit tests** per module: one file per module plus `mod tests` blocks for in-module assertions.
- **Layout tests**: `tests/layout.rs` integration test that runs `static_assertions` and `memoffset` checks; failures fail compilation, not the test process.
- **Serde round-trip tests**: `tests/serde_round_trip.rs` (host feature) — every host-side type serializes to JSON and deserializes back to a structurally-equal value.
- **Negative tests**: `tests/negative.rs` — invalid magic bytes, unknown discriminants, malformed semantic ids, non-zero reserved bytes.
- **Exhaustive enum tests**: `tests/exhaustive.rs` — every `ALL` constant is asserted to contain every variant exactly once via `strum::EnumCount` or a hand-written variant counter.
- **Cross-module property tests**: e.g. `liveness::record_progress_then_idle_frame_property` (proptest).
- **No-local-unsafe test**: `#![forbid(unsafe_code)]` is present in `lib.rs`, and CI runs `cargo check -p gbf-abi --all-features` with that lint active. The compiler is the authoritative gate. A textual `grep -R "unsafe" gbf-abi/src` may be used as a supplemental check, but it must filter out the `forbid(unsafe_code)` declaration itself, which legitimately contains the substring.

### 6.2 Property tests

Two property tests with explicit deterministic seeds:

```rust
#[test]
fn liveness_progress_epoch_monotone() {
    proptest!(|(events: Vec<LivenessEvent>)| {
        let mut counters = LivenessCounters::new(60);
        let mut last_epoch = 0;
        for event in &events {
            event.apply(&mut counters);
            assert!(counters.progress_epoch >= last_epoch);
            last_epoch = counters.progress_epoch;
        }
    });
}

#[test]
fn schema_resolve_round_trip() {
    proptest!(|(schema: SemanticCheckpointSchema)| {
        for entry in &schema.checkpoints {
            assert_eq!(schema.resolve_semantic(&entry.semantic), Some(entry.compact));
            assert_eq!(schema.resolve_compact(entry.compact), Some(&entry.semantic));
        }
    });
}
```

### 6.3 Determinism

A single `cargo test --features host` run is deterministic given the proptest seed. The crate has no `SystemTime`, `Instant::now`, `rand`, or `thread_rng` use anywhere in production paths or tests (proptest seeds are set via env or file).

## 7. Dependencies

### 7.1 New direct dependencies

| Dependency           | Purpose                                  | Where used | Feature flags |
|----------------------|------------------------------------------|------------|---------------|
| `static_assertions`  | `const_assert_eq!`                       | every layout module | always |
| `memoffset`          | `offset_of!` macro                       | every layout module | always |
| `proptest`           | property tests                           | dev-dependency only | none |
| `serde_json`         | host-side round-trip                     | dev/test only | already in tree |

`static_assertions` and `memoffset` are ordinary compile dependencies, not runtime logic. They emit no runtime code, but they must be available while compiling production modules because the assertions live next to the ABI type definitions.

### 7.2 Cargo.toml diff

```toml
[features]
default = ["host"]
host    = ["std", "alloc"]
std     = ["serde/std"]
alloc   = ["serde/alloc"]

[dependencies]
gbf-foundation = { path = "../gbf-foundation" }
serde          = { workspace = true, default-features = false, features = ["derive"] }
static_assertions = "1"
memoffset      = "0.9"

[dev-dependencies]
proptest       = "1"
serde_json     = { workspace = true }
```

## 8. Constitutional grounding

| Constitution clause | F-A3 gate |
|---------------------|-----------|
| §I.1 (correctness by construction) | every enum carries `ALL`; layout asserts pin field offsets; discriminants pinned |
| §III (shifting left)               | layout assertions are compile-time, not runtime |
| §IV.3 (reproducible builds)        | every host-side type has a serde round-trip test |
| §V.3 (silence on success, loud on failure) | typed errors per §5; FaultSnapshot carries register state |
| §VI.1 (single source of truth)     | re-exports collapse the public surface; no shadow types in `gbf-runtime` |
| §I.2 (`unsafe` forbidden)          | `#![forbid(unsafe_code)]` in `lib.rs` |

## 9. Tasks and ordering

### 9.1 Task graph

```
T-A3.1 (version)
   │
   ├── T-A3.2 (continuation + liveness)   ← reads AbiVersion; needs CompactCheckpointId for liveness counter, but CompactCheckpointId is a u16 newtype that does not require T-A3.6 to land first
   ├── T-A3.3 (harness)
   ├── T-A3.4 (fault)
   ├── T-A3.5 (interrupt)
   ├── T-A3.6 (checkpoint)                ← introduces SemanticCheckpointSchema; T-A3.2 only needs CompactCheckpointId, which can be defined in T-A3.6 as a u16 newtype published before the schema type
   └── T-A3.7 (trace)                     ← reads CompactCheckpointId, SliceId, TraceProbeId
```

There is a small inverted dependency: `LivenessCounters` (T-A3.2) carries a `CompactCheckpointId` field, and the canonical home for that type is T-A3.6. The resolution: **T-A3.6 lands the `CompactCheckpointId` newtype and `SemanticStratum` enum first**, then T-A3.2 lands `LivenessCounters` and `InferenceStateHeader`, then T-A3.6 fills in `SemanticCheckpointId` (host) and `SemanticCheckpointSchema` (host). Beads' `bd-1ml` already depends on `bd-2ul` (T-A3.1); the order T-A3.1 → T-A3.6 (compact id only) → T-A3.2 → T-A3.6 (schema) → T-A3.4 → T-A3.5 → T-A3.7 → T-A3.3 is a valid linearization.

### 9.2 PR shape

**The entire RFC ships in a single PR.** All seven tasks (T-A3.1 through T-A3.7) land together: `AbiVersion` + `BuildIdentityBlock`, `InferenceStateHeader` + `LivenessCounters`, `HarnessCommandBlock` + `HarnessResultBlock`, `FaultCode` + `FaultDomain` + `FaultSnapshot`, `InterruptPolicy` + `ResourceLease`, `SemanticCheckpointId` + `CompactCheckpointId` + `SemanticCheckpointSchema`, `TraceEvent` + `TraceBudget`. Total ~1,500 LOC of production code + ~1,600 LOC of tests, plus the F-A3 review packet (§14).

Three reasons the contract crate must land atomically rather than across stacked PRs:

1. **Layout assertions are cross-cutting.** `layout-report.json`, `tests/layout.rs`, and the `static_assertions` blocks in `lib.rs` and the per-module sources reference every `#[repr(C)]` type in the crate. Splitting the crate across PRs means each interim PR ships an incomplete `layout-report.json` and an incomplete layout-evidence document — neither is reviewable on its own.
2. **The inverted dependency between T-A3.2 and T-A3.6 has no clean PR boundary.** `LivenessCounters` (T-A3.2) carries a `CompactCheckpointId` (T-A3.6); the §9.1 linearization handles this *within* the crate by landing the `CompactCheckpointId` newtype before `LivenessCounters` and the `SemanticCheckpointSchema` after. That ordering is a file-write ordering, not a PR boundary; trying to express it as two PRs forces an artificial `compact-id stub` PR whose only job is to satisfy the next PR.
3. **No external consumer exists yet.** F-A4, F-A5, F-H3 and the oracle stack all consume `gbf-abi` types, but none of them is in tree at start of F-A3 (per §2.3). There is no caller demanding partial delivery, so the only thing a multi-PR split would buy is review-overhead amortization — which the single review packet (§14) already addresses.

The single PR ships with the full F-A3 review packet (§14). The packet's contents are specified in §14 as content areas the engineer must cover; the file layout, scripts, formats, and exact artifact set are the engineer's call once the implementation is in hand.

The §9.1 task-graph linearization (T-A3.1 → T-A3.6 compact-id portion → T-A3.2 → T-A3.6 schema portion → T-A3.4 → T-A3.5 → T-A3.7 → T-A3.3) is the **commit ordering inside this single PR**, not a PR sequence. Engineers picking up F-A3 should land commits in that order so each commit individually compiles, tests pass at every commit, and `git bisect` stays useful — but the merge into `main` is one PR and one squash (or one merge commit) per the project's normal cadence.

## 10. Risk register

| Risk | Mitigation |
|------|------------|
| `#[repr(C)]` size differs across compilers | `static_assertions::const_assert_eq!` on size + per-field offset; CI runs nightly + stable + msrv |
| `#[repr(u16)]` enum discriminant shifts | Discriminants explicitly pinned (`= 0x0010`); `ALL` array test fails on missing variant |
| Schema-version skew between runtime and host | `BuildIdentityBlock::semantic_schema_version` carried at runtime; harness rejects mismatched schemas |
| Magic bytes accidentally collide with existing data | `"GBLM"`/`"HCMD"`/`"HRES"` are byte sequences unlikely in default-zero memory; F-A5 always writes them at fixed addresses; host parsing ranges over a known address only |
| Liveness threshold of 0 disables the check silently | `Bringup` profile is the only legal user; F-A5 enforces non-zero in `Default`/`Trace`/`Recovery` |
| `progress_epoch` saturation in long sessions | Saturating arithmetic preserves monotonicity; once at `u32::MAX` the runtime issues a session-end checkpoint and resets via `LivenessCounters::reset` |
| `Cow<'static, str>` lifetime mismatch | `from_owned(String)` returns `Cow::Owned`; `from_static(&'static str)` returns `Cow::Borrowed`; both validated by the same parser |
| Hand-rolled `from_le_bytes` parsers drift from the canonical layout | `tests/layout.rs` regenerates a 152-byte fixture via `BuildIdentityBlock::to_bytes` and asserts every named offset round-trips through `from_bytes`; any drift surfaces as a test failure, not undefined behavior |
| Schema-only `gbf-abi` accidentally grows runtime logic | `#![forbid(unsafe_code)]` + `#![cfg_attr(not(feature = "std"), no_std)]` bound the surface; clippy rule `clippy::missing_safety_doc` is moot since there is no `unsafe` |

## 11. Tasks T-A3.1..T-A3.7 (deep-dive cross-reference)

Every child task in beads has a one-paragraph entry below mapping to the §3 design.

### 11.1 T-A3.1 (`bd-2ul`)

Owns: `AbiVersion`, `CompatibilityEnvelope` (host), `BuildIdentityBlock`. See §3.1.

Acceptance gates per bead comment plus F-A3 additions:

- `cargo check -p gbf-abi`
- `cargo test -p gbf-abi -- version::current_constant_set`
- `cargo test -p gbf-abi -- version::compatibility_envelope_no_self`
- `cargo test -p gbf-abi -- version::build_identity_layout`
- *F-A3 additions*: `version::build_identity_offsets`, `version::build_identity_constructor_sets_magic`, `version::build_identity_constructor_zeroes_reserved`, `version::build_identity_validate_rejects_bad_magic`, `version::build_identity_validate_rejects_nonzero_reserved`, `version::build_identity_from_bytes_round_trip`, `version::semver_round_trip`.

### 11.2 T-A3.2 (`bd-2qx`)

Owns: `LivenessCounters` (`liveness.rs`), `InferenceStateHeader` (`continuation.rs`), `FaultCodeOptional`. See §3.2.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- continuation::header_layout`
- `cargo test -p gbf-abi -- liveness::livelock_threshold`
- `cargo test -p gbf-abi -- liveness::progress_advance`
- *F-A3 additions*: `liveness::idle_frames_saturate`, `liveness::progress_epoch_saturates`, `continuation::split_header_tail_validates_size`.

### 11.3 T-A3.3 (`bd-2m8`)

Owns: `HarnessCommandBlock`, `HarnessResultBlock`, `HarnessOp`, `HarnessResultKind`. See §3.3.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- harness::layout`
- `cargo test -p gbf-abi -- harness::op_kind_complete`
- `cargo test -p gbf-abi -- harness::constructor_sets_magic`
- *F-A3 additions*: `harness::op_from_u16_rejects_8`, `harness::doorbell_constants_distinct`.

### 11.4 T-A3.4 (`bd-1si`)

Owns: `FaultCode`, `FaultDomain`, `FaultSnapshot`, `RegisterSnapshot`, `RecoveryAction`, `FaultPolicy` (host), `BootValidationPlan`. See §3.4.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- fault::code_to_domain_total`
- `cargo test -p gbf-abi -- fault::policy_default_action`
- `cargo test -p gbf-abi -- fault::snapshot_layout`
- *F-A3 additions*: `fault::range_partition`, `fault::register_snapshot_layout`, `fault::all_unique_discriminants`.

### 11.5 T-A3.5 (`bd-30s`)

Owns: `InterruptPolicy`, `ResourceLease`, `ResourceLeaseKind`, `RomWindowBinding`, `SramPageBinding`, `LeaseId`, `SliceId`, `OverlayId`. See §3.5.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- interrupt::policy_enum_complete`
- `cargo test -p gbf-abi -- interrupt::lease_yield_safety_table`
- `cargo test -p gbf-abi -- interrupt::lease_id_uniqueness`
- *F-A3 additions*: `interrupt::lease_id_distinct_from_slice_id`, `interrupt::balanced_predicate`, `interrupt::sram_page_disabled_state`.

### 11.6 T-A3.6 (`bd-1ml`)

Owns: `SemanticCheckpointId`, `CompactCheckpointId`, `SemanticStratum`, `SemanticCheckpointSchema` (host), `CheckpointEntry` (host). See §3.6.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- checkpoint::serde_round_trip`
- `cargo test -p gbf-abi -- checkpoint::compact_id_unique`
- `cargo test -p gbf-abi -- checkpoint::stratum_exhaustive`
- *F-A3 additions*: `checkpoint::semantic_id_validation_basic`, `checkpoint::semantic_id_rejects_uppercase`, `checkpoint::schema_rejects_compact_zero`, `checkpoint::group_by_stratum_partitions`.

### 11.7 T-A3.7 (`bd-34v`)

Owns: `TraceEvent`, `TraceProbeId`, `ProbeLevel`, `ProbeBudgetClass`, `TraceBudget`, `TraceDropPolicy`. See §3.7.

Acceptance gates plus additions:

- `cargo test -p gbf-abi -- trace::event_layout`
- `cargo test -p gbf-abi -- trace::probe_level_exhaustive`
- `cargo test -p gbf-abi -- trace::drop_policy_exhaustive`
- *F-A3 additions*: `trace::trace_budget_constructor`, `trace::serde_round_trip` (full coverage of all variants).

## 12. Claim-to-gate matrix (closure-style)

| Claim | Gating test / artifact |
|-------|------------------------|
| `CURRENT_ABI` is non-zero (uninitialized memory cannot pass) | `version::current_constant_set` |
| `AbiVersion` ordering matches lex order on (major, minor, patch) | `version::ord_total` |
| `AbiVersion` round-trips through `gbf_foundation::SemVer` for in-bounds values | `version::semver_round_trip` |
| `BuildIdentityBlock` is exactly 152 bytes with pinned per-field offsets | `version::build_identity_layout` + `version::build_identity_offsets` (compile-time `static_assertions`) |
| `BuildIdentityBlock::new` stamps magic `b"GBLM"` | `version::build_identity_constructor_sets_magic` |
| `BuildIdentityBlock::new` zeroes reserved fields; `validate` rejects non-zero | `version::build_identity_constructor_zeroes_reserved` + `version::build_identity_validate_rejects_nonzero_reserved` |
| `BuildIdentityBlock::validate` rejects bad magic | `version::build_identity_validate_rejects_bad_magic` |
| `BuildIdentityBlock` from_bytes/to_bytes round-trips | `version::build_identity_from_bytes_round_trip` |
| `CompatibilityEnvelope` rejects `current` in `backward_compatible_with` | `version::compatibility_envelope_no_self` |
| `CompatibilityEnvelope::accepts` matches its declared rules | `version::compatibility_envelope_validate` |
| `LivenessCounters` is 12 bytes, 4-byte-aligned | `liveness::layout` |
| `record_progress` bumps epoch and resets idle frames | `liveness::progress_advance` |
| `note_idle_frame` saturates at `u16::MAX` | `liveness::idle_frames_saturate` |
| `progress_epoch` saturates at `u32::MAX` | `liveness::progress_epoch_saturates` |
| Threshold of 0 disables livelock detection | `liveness::livelock_threshold_zero_disables` |
| `is_livelocked` fires at `>=` threshold, not strictly `>` | `liveness::livelock_threshold_fires_at_eq` |
| `InferenceStateHeader` is 32 bytes with pinned offsets | `continuation::header_layout` |
| `split_header_tail` enforces tail length from `BuildIdentityBlock` | `continuation::split_header_tail_validates_size` |
| Every `HarnessOp` discriminant is unique and covered by `ALL` | `harness::op_kind_complete` |
| `HarnessOp::from_u16` rejects unknown discriminants | `harness::op_from_u16_rejects_8` |
| Both block layouts are 44 bytes with pinned offsets and constructor-stamped magic | `harness::layout` + `harness::constructor_sets_magic` + `harness::validate_rejects_bad_magic` |
| Doorbell raised/clear constants are distinct | `harness::doorbell_constants_distinct` |
| Every `FaultCode::ALL` member has a `FaultDomain` | `fault::code_to_domain_total` |
| Every `FaultCode` discriminant falls inside its declared domain range | `fault::range_partition` |
| `FaultSnapshot` is 36 bytes with pinned offsets | `fault::snapshot_layout` |
| `FaultPolicy::default_action` is not `BootValidationOnly` | `fault::policy_default_action_validation` |
| `RecoveryAction` enumeration is exhaustive | `fault::recovery_action_exhaustive` |
| `InterruptPolicy::ALL` covers exactly 3 variants | `interrupt::policy_enum_complete` |
| `ResourceLeaseKind::yield_safe` returns the documented per-variant value | `interrupt::lease_yield_safety_table` (table-driven) |
| `LeaseId`/`SliceId`/`OverlayId` newtypes do not coerce | `interrupt::lease_id_distinct_from_slice_id` |
| `ResourceLease::is_balanced` enforces acquired ≤ released | `interrupt::balanced_predicate` |
| `SemanticCheckpointId` parser rejects uppercase, leading dot, double dot | `checkpoint::semantic_id_validation_basic` + `_rejects_uppercase` + `_rejects_leading_dot` + `_rejects_double_dot` |
| `CompactCheckpointId::NONE = 0` is reserved | `checkpoint::compact_none_sentinel` + `checkpoint::schema_rejects_compact_zero` |
| `SemanticStratum::ALL` is exactly the three planv0.md strata | `checkpoint::stratum_exhaustive` |
| `SemanticCheckpointSchema` has unique `compact` and `semantic` ids | `checkpoint::schema_validates_unique_compact` + `_unique_semantic` |
| `resolve_compact` and `resolve_semantic` round-trip | `checkpoint::schema_resolve_round_trip` |
| `group_by_stratum` partitions exactly | `checkpoint::group_by_stratum_partitions` |
| `TraceEvent` is 32 bytes with pinned offsets (no compiler-inserted padding surprises) | `trace::event_layout` |
| `ProbeLevel`/`ProbeBudgetClass`/`TraceDropPolicy` are exhaustive | `trace::*_exhaustive` |
| `TraceBudget::new` rejects internally-inconsistent budgets (zero events with non-zero bytes, or non-zero events with zero bytes) | `trace::trace_budget_constructor_rejects_inconsistent` + `trace::trace_budget_constructor_accepts_zero_zero` |
| Every host-side type round-trips through serde JSON | `*::serde_round_trip` (per-module) |
| `gbf-abi` introduces zero `unsafe` lines | `grep -R "unsafe" gbf-abi/src` returns nothing; CI gate |
| `#[forbid(unsafe_code)]` is in `lib.rs` | `lib.rs` line check (CI step) |

## 13. References

### 13.1 Internal

- `history/planv0.md` — line 271 (gbf-abi ownership), line 1795 (liveness contract), line 2170 (auto-yielding ABI), line 2333 (continuation/liveness counters), line 2398 (fault codes), line 2533/2552 (identity handshake), line 2554 (revision-pass items 7+8).
- `CONSTITUTION.md` — §I.1, §III, §IV.3, §V.3, §VI.1.
- `.agents/skills/qat-bead-closure/SKILL.md` — closure-skill checklist (claim-to-gate matrix, no future variant rule).
- `bd-2k2` (F-A3 feature bead) and child tasks `bd-2ul`, `bd-2qx`, `bd-2m8`, `bd-1si`, `bd-30s`, `bd-1ml`, `bd-34v`.
- Existing source: `gbf-abi/src/lib.rs` (module stub list), `gbf-foundation/src/semver.rs` (`SemVer`), `gbf-artifact/src/ids.rs` (shared `Hash256`).

### 13.2 External

- Pan Docs cartridge header: <https://gbdev.io/pandocs/The_Cartridge_Header.html>
- Pan Docs Interrupt Sources: <https://gbdev.io/pandocs/Interrupt_Sources.html>
- Pan Docs MBC5: <https://gbdev.io/pandocs/MBC5.html>
- `bytemuck` crate (Pod/Zeroable derives — referenced as a *non-dependency* the crate intentionally does not use): <https://docs.rs/bytemuck>
- `static_assertions` crate: <https://docs.rs/static_assertions>
- `memoffset` crate: <https://docs.rs/memoffset>
- `proptest` crate: <https://docs.rs/proptest>

## 14. Review packet requirements

The F-A3 review packet is the engineer's pre-digestion of this RFC for the reviewer. Its job is to let the reviewer verify every load-bearing claim without having to re-derive the design from the diff. **The contents below are mandatory; the file layout, scripts, formats, diagram tools, and exact artifact set are the engineer's call once the implementation is in hand and they can see what naturally falls out of the code.**

### 14.1 Required content areas

The packet must cover each of the following. Each item describes *what* must be conveyed, not *how* the engineer chooses to deliver it.

1. **Orientation.** Short landing page pointing at this RFC, the closed beads, the scope, and a recommended reading path.
2. **Scope ledger.** What is in F-A3 and what is explicitly deferred. Each deferred item names: the deferred subject; why it is not in F-A3; the owning feature/bead; and the F-A3 guard (test or type) that prevents accidental dependence.
3. **Reading-order / multi-pass guide.** A layered walkthrough so a reviewer doesn't have to hold the whole crate in one pass. At minimum the passes should cover: layout invariants → enum exhaustiveness/discriminant pinning → liveness semantics → checkpoint schema → harness/trace → host-side surfaces.
4. **Diff map.** One row per touched file with a risk rating, a one-line reason a reviewer should care, and the load-bearing tests that gate the file.
5. **Architecture diagrams.** At minimum: a crate-relationship map (this crate vs. its consumers); the byte layouts for `BuildIdentityBlock` and `InferenceStateHeader`; the harness command/result doorbell sequence; the `FaultCode` → `FaultDomain` partition; the three checkpoint strata. Tooling and rendering choices are the engineer's call.
6. **Correctness dossier.** For every `#[repr(C)]` type: the layout invariants enforced. For every `#[repr(uN)]` enum: the discriminant table, `ALL`-coverage, and the `from_uN` rejection rule. For the `SemanticCheckpointId` parser: the rejection rules. For `SemanticCheckpointSchema`: the uniqueness and `CompactCheckpointId(0)` reservation invariants. For `LivenessCounters`: the saturation and threshold rules.
7. **Claim-to-gate mapping.** Every load-bearing RFC claim (start from §12) mapped to at least one gating test, type invariant, or generated artifact.
8. **Test coverage report.** What tests exist, what they assert, and how to run them in each feature configuration (`--no-default-features`, `--no-default-features --features alloc`, `--features host`).
9. **Reproducibility report.** Pinned toolchain, lockfile, host triple; deterministic-build evidence; how `SOURCE_DATE_EPOCH` flows into `BuildIdentityBlock::timestamp_unix`.
10. **Layout evidence.** Actual byte-by-byte breakdown of every `#[repr(C)]` type, cross-referenced to the source.
11. **Generated-artifacts guide.** For each generated artifact (such as a layout report, a `SemanticCheckpointSchema` fixture, a `BuildIdentityBlock` byte fixture, a `FaultCode` → `FaultDomain` table), what it is, how it was built, and how to regenerate. The exact set of artifacts is the engineer's call; it must be sufficient to verify §6's invariants.
12. **Dependency report.** Full dependency tree, feature gates, evidence of no upward dependency on `gbf-runtime` / `gbf-codegen` / `gbf-asm` / `gbf-artifact`, license summary.
13. **Known-debt ledger.** Every TODO/FIXME/punt with owner and removal condition.
14. **Out-of-scope ledger.** Every item explicitly deferred to a downstream feature, named with the owning feature (mirrors §1.2). Each entry uses the same four-field block as the scope ledger (item / why / owner / F-A3 guard).
15. **API guide.** What is public, what is host-feature-gated, what is stable, what is evolving.
16. **Error-shape report.** Every typed error variant from §5, what triggers it, and how the caller is expected to recover.
17. **Reviewer checklist.** A single-page tickbox covering the load-bearing invariants a reviewer must confirm before approving.
18. **Source-to-artifact traceability.** For any generated artifact, a link from the bytes back to the source code (and exact command) that produced them.
19. **Optional supplemental videos.** Short walkthroughs with transcripts and exact reproduction commands. The engineer decides whether the complexity warrants them; if shipped, they supplement the written packet, never replace it.

### 14.2 Reproducibility (the one hard rule)

The whole packet must be regenerable from a clean checkout by running a single command. The script's name, location, and implementation are the engineer's call; the contractual requirement is that every checked-in artifact is reproducible deterministically and that staleness fails loudly.

### 14.3 Acceptance bar

The packet is complete only when:

- A fresh checkout regenerates the packet successfully via the single regen command.
- Every load-bearing RFC claim maps to a test, type invariant, or generated artifact.
- The packet pre-digests layout, discriminant pinning, schema invariants, and test strategy — a reviewer should not have to rediscover these from the diff.
- Every `#[repr(C)]` type, every `#[repr(uN)]` enum, and every parser invariant from §3 is gated by at least one test in the packet.
- The "no `unsafe`" invariant is verifiable from the packet (`#![forbid(unsafe_code)]` plus grep evidence).
- The "no upward dependency" invariant is verifiable from `cargo tree`.
- Each in-scope content area in §14.1 is covered.

### 14.4 Core principle

> The engineer should not make the reviewer rediscover the layout, discriminant pinning, schema invariants, or test strategy from the diff. The packet pre-digests all of that, while still giving the reviewer enough precise links, commands, and evidence to independently verify every claim.

The form of that pre-digestion — directory layout, file formats, script names, diagram tools, video format, even the exact set of generated artifacts — is the engineer's call after the implementation lands and they can see what the code naturally produces.

## 15. Appendix: file-by-file change set

| File                                  | Change             | Lines (est.) |
|---------------------------------------|--------------------|-------------:|
| `gbf-abi/src/lib.rs`                  | Replace stub: feature gates, re-exports, forbid unsafe | ~60          |
| `gbf-abi/src/version.rs`              | New (replace stub) | ~250         |
| `gbf-abi/src/continuation.rs`         | New (replace stub) | ~150         |
| `gbf-abi/src/liveness.rs`             | New (replace stub) | ~150         |
| `gbf-abi/src/harness.rs`              | New (replace stub) | ~200         |
| `gbf-abi/src/fault.rs`                | New (replace stub) | ~350         |
| `gbf-abi/src/interrupt.rs`            | New (replace stub) | ~200         |
| `gbf-abi/src/checkpoint.rs`           | New (replace stub) | ~300         |
| `gbf-abi/src/trace.rs`                | New (replace stub) | ~150         |
| `gbf-abi/tests/layout.rs`             | New (integration)  | ~200         |
| `gbf-abi/tests/serde_round_trip.rs`   | New                | ~250         |
| `gbf-abi/tests/exhaustive.rs`         | New                | ~150         |
| `gbf-abi/tests/negative.rs`           | New                | ~250         |
| `gbf-abi/tests/property.rs`           | New (proptest)     | ~200         |
| `gbf-abi/Cargo.toml`                  | Add features + deps | +30          |
| Review packet (per §14)               | New (paths, scripts, examples, docs, diagrams chosen by the engineer once the implementation lands) | (engineer's call) |

**Total implementation surface (excluding the engineer-shaped review packet): ~3000 LOC, ~50% of which is tests.**

## 16. End

This RFC stays inside the F-A3 boundary. Anything that requires F-A4's runtime ABI, F-A5's runtime nucleus, F-D1/D2/D3's runtime/host plumbing, or `gbf-codegen`'s schema emission is explicitly deferred. The proposal lets F-A3 close without those features existing, while leaving every seam (`ResourceLeaseKind`, `SemanticCheckpointSchema`, `BuildIdentityBlock`, `FaultPolicy`, `TraceEvent`) shaped for them to plug in cleanly.

Resolved decisions from the prior review pass:

1. **Reserved-byte testing strategy.** Required gates are: every constructor zeroes reserved bytes (per-byte assertion in test); every decoder/`validate` rejects non-zero reserved bytes; byte fixtures show reserved offsets are zero. A struct-level `Default::default() == zero_init()` test is *not* sufficient because most ABI structs have non-zero defaults (magic bytes, `CURRENT_ABI`).
2. **`AbiVersion` is split from `gbf-foundation::SemVer`.** The ABI version is a ROM-resident binary field, not a general semver type. A 3-byte `u8/u8/u8` fits the actual version space and gives a stable size of 3 bytes for layout pinning. `from_semver` is fallible and returns `AbiVersionError::SemVerOutOfRange` when any component exceeds 255.
3. **`continuation_tail_bytes` belongs inside `BuildIdentityBlock`.** Changing the tail length changes the executable runtime state layout, so the field is identity-relevant. The companion rule is the schema-pairing requirement: a `SemanticCheckpointSchema` sidecar must match a runtime by `build_hash` and `compile_request_hash`, not just `semantic_schema_version` (see §3.6.3).
4. **`HarnessOp` does not gain a `Custom { tag: u16 }` variant.** The block stores `op: u16`, so extension is represented as **reserved discriminant ranges**, not as a typed Rust variant with a payload:

   | Range            | Owner |
   |------------------|-------|
   | `0x0000..=0x007F`| Core ops defined by F-A3 |
   | `0x0080..=0xBFFF`| Reserved for future F-D2 / harness extension |
   | `0xC000..=0xFFFF`| Experimental / vendor-specific |

   Unknown core-range ops decode to `Err(HarnessProtocolError::UnknownOp { raw })`. F-D2 may opt in to ops in the reserved range only after they are added to `HarnessOp::ALL`.
5. **`FaultDomain` carries an explicit `range()` method.** Per the §3.4.2 patch, `FaultDomain` is `#[repr(u16)]` with pinned discriminants and a `pub const fn range(self) -> RangeInclusive<u16>` accessor. The `range_partition` test asserts both directions (every code maps to a domain; every code's discriminant falls inside that domain's declared range).
6. **F-A3 owns a minimal `TraceProbeRegistry` shell.** Just enough to prevent drift between probe ID, level, budget class, and a *named* schema tag. Actual payload schemas remain F-D3:

   ```rust
   #[cfg(feature = "host")]
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct TraceProbeRegistry {
       pub abi_version: AbiVersion,
       pub build_hash:  [u8; 32],
       pub probes:      alloc::vec::Vec<TraceProbeEntry>,
   }

   #[cfg(feature = "host")]
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   pub struct TraceProbeEntry {
       pub probe:              TraceProbeId,
       pub level:              ProbeLevel,
       pub budget_class:       ProbeBudgetClass,
       pub payload_schema_tag: alloc::borrow::Cow<'static, str>,
   }
   ```

   F-A3 does not store the schema *content*; F-D3 owns per-tag interpretation.

Open items still worth reviewer input:

1. Should `BuildIdentityBlock::validate` require `semantic_schema_version >= 1`, or accept `0` as a "no schema" sentinel? Current text requires `>= 1`; flag if F-D3 wants the `0` path.
2. Is the eight-bit `_reserved0` immediately after `abi` the right choice, or should the trailing pad migrate to before `timestamp_unix` so the identity block has a tighter prefix? Either layout is reproducible; the current choice keeps the four hashes 8-byte-aligned.
