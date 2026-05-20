# RFC F-B13: Schedule + Proof — `GbSchedIR` (Stage 10) and `ResourceStateValidation` (Stage 10.5)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B13 implementation. `history/planv0.md`
remains the architectural context document, but this RFC is allowed to refine,
narrow, or supersede `planv0.md` wherever this RFC makes a more precise
implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence
must be recorded in an `Amends planv0` note close to the relevant decision.
This is not a request to edit `planv0.md` immediately; it is a local
source-of-truth ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B13 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, `StageCache` key construction, `ReportEnvelope` shape), the
  F-B2/F-B4 RFC wins. F-B13 inherits those surfaces unchanged unless this
  RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on `QuantGraph` or
  `GbInferIR` shape or canonical-product handling, the F-B3/F-B5 RFC wins.
* If this RFC and `F-B11-F-B12-overlay-arena-plans.md` disagree on
  `OverlayId`, `ArenaSlot`, `NamedArena`, or `OverlayInstall` shape, the
  F-B11/F-B12 RFC wins. F-B13 consumes those types verbatim.
* F-B6 (`ObservationPlan`), F-B7 (`RangePlan`), F-B8 (`StoragePlan`), F-B9
  (`SramPagePlan`), and F-B10 (`RomWindowPlan`) RFCs are forthcoming /
  recently landed. This RFC consumes their public products and reportable
  identities by hash; if a forthcoming RFC changes those public types,
  that RFC must explicitly amend this RFC.
* If a later RFC changes any public type, report shape, cache key,
  diagnostic code, or canonicalization rule introduced here, that later
  RFC must explicitly amend this RFC. In particular: F-B14
  (`ScheduleCostAnalysis`), F-B15 (Backend including
  `ReachabilityValidation`), and F-B16 (`FeasibilityRefinementLoop`) all
  consume `SchedulePack`; any of those RFCs that needs to change a
  `SchedSlice`, `ResourceLease`, `ResidencyEpoch`, `SchedulePack`,
  `ModeSwitchPolicy`, `RuntimeDriftMonitor`, `DriftEnvelope`, or
  certificate-body shape must amend this RFC by name.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature beads   | bd-9ae **F-B13 GbSchedIR + ResourceStateValidation (Stages 10, 10.5)** |
| Open tasks      | To be minted: T-B13.1..T-B13.N (residency-epoch construction, slice formation, lease balance proof, lease-flow analysis, mode-pack assembly, drift-envelope binding, semantic-checkpoint pinning, `sched_ir.json` emitter, `slice_report.json` emitter, `certs/resource_state.cert.json` emitter, schema/round-trip tests, K10 + K10.5 StageCache wiring) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 113–212 (target, regions, banks, MBC5); 1665–1770 (Stages 6, 7, 8, 8.5, 9 inputs); 1770–1900 (Stage 10 GbSchedIR + Stage 10.5 ResourceStateValidation bodies — `SchedSlice`, `ResourceLease`, `ResourceVector`, `ResidencyEpoch`, `SchedulePack`, `ModeSwitchPolicy`, `RuntimeDriftMonitor`, `DriftEnvelope`, `DriftTrigger`, `YieldKind`, `InterruptPolicy`); 1894–1990 (Stage 11 + Backend preamble); 1989–2210 (runtime architecture, banking, persistence, cooperative scheduling, ISR rules); 2640–2870 (tests, certs, reports/artifacts: `resource_state.cert.json`) |
| Glossary        | `history/glossary.md` (slice, yield, liveness, BankLease, residency, common bank, expert bank, Bank0, WRAM overlay, page state, commit group, SchedulePack, RuntimeMode, drift, semantic checkpoint, interrupt policy) |
| Constitution    | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ReportEnvelope`, `ValidationDiagnostic`, canonical JSON / self-hash, StageCache key construction, `ResolvedCompilePolicy` provenance vocabulary); F-B3/F-B5 Canonical IRs (provides `QuantGraph`, `GbInferIR` consumed transitively for value/effect/checkpoint identity); F-B6 ObservationPlan (provides semantic-checkpoint and trace-probe pin set); F-B7 RangePlan (provides reduction-loop tile sizes and reduction-site identity); F-B8 StoragePlan (provides `Materialization`, `LifetimeClass`, `AliasClassId`); F-B9 SramPagePlan (provides `SramPageBinding` and page-switch budgets); F-B10 RomWindowPlan (provides `RomWindowBinding` and `KernelResidency`); F-B11 OverlayPlan (provides `OverlayId` and `OverlayInstall`); F-B12 ArenaPlan (provides `ArenaSlot` byte ranges); F-B14 ScheduleCostAnalysis (consumes `SchedulePack`); F-B15 Backend including `ReachabilityValidation` (computed version of the residency claims this RFC checks against annotations); F-B16 FeasibilityRefinementLoop (BLOCKED on oracle); F-B17 StageCache integration sweep; F-A4 BankLease/BankGuard ABI (lease-acquire/release semantics this RFC mirrors); F-A5 Bank0 runtime (cooperative-kernel conventions and ISR-residency rules); F-D1 cooperative-kernel scheduler (consumes `SchedulePack` and `RuntimeDriftMonitor` at runtime) |
| Sister deps     | F-C3 ScheduleOracle (consumes `SchedulePack` for op-for-op correspondence at slice boundaries); F-D6 SchedulePack Mode Switching (BLOCKED on oracle); F-F2 Certificates (consumes `certs/resource_state.cert.json`) |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B13 sits inside the
compiler-pipeline epic, where that epic sits inside the full project, and
which adjacent chunks' contracts this RFC inherits or honors.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"; bead-side mirror in `Epic *: …` issues):

```text
Epic A — M0 Foundation Stack
          gbf-asm, gbf-hw, gbf-abi, BankLease/BankGuard, Bank0 runtime,
          gbf-emu, gbf-debug, gbf-store. Provides the target/abi/asm
          contracts every other epic builds on. Status: substantially
          merged on main; F-A6 (gbf-store + StageCache) CLOSED.

Epic B — Compiler Pipeline (14 stages + refinement loop)        ← THIS EPIC
          The transform pipeline from frozen ArtifactCore +
          CompileRequest to a CompiledBuild (ROM + reports + certificates).
          Where most of M1–M3 lives.

Epic C — Oracle Stack
          DenotationalOracle (F-C1), ArtifactOracle (F-C2),
          ScheduleOracle (F-C3), ConformanceEnvelope (F-C4).
          F-C3 consumes the `SchedulePack` this RFC produces.

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack
          mode switching. Consumes the runtime side of the
          `RuntimeDriftMonitor` / `DriftEnvelope` handshake this RFC pins.

Epic E — Calibration & Bench
          gbf-bench: cycle calibration, kernel timing, autotune.
          F-B14 consumes calibration; F-B13 does NOT — see §1.5.

Epic F — Reports & Verify
          gbf-report (build reports, certificates) + gbf-verify
          (independent slow reference implementations). Consumes
          `certs/resource_state.cert.json`.

Epic G — Data, Lexical, Decode Pipeline
          gbf-data (corpus, charset, normalization, decode policy).

Epic H — Kernel
          gbf-kernel (KernelSpec + matvec/residual/norm/route/decode kernel
          implementations). F-B13 references `KernelSpecId` opaquely.
```

### 0.2 Epic B's anatomy — the 14-stage pipeline plus loop

Per `planv0.md` §"The compiler pipeline," Epic B has 14 numbered stages
bracketed by a **policy/feasibility envelope**, a **transform pipeline**,
and a **reporting envelope**, plus a bounded `FeasibilityRefinementLoop`
that wraps stages 5–11.

```text
Policy / feasibility envelope:
  F-B2  Stages 0, 0.5  ArtifactValidationAndUpgrade + ResolvedCompilePolicy
  F-B3  Stage 1        QuantGraph
  F-B4  Stage 2        StaticBudgetReport

Transformative stages (wrapped by FeasibilityRefinementLoop):
  F-B5  Stage 3        GbInferIR (value/effect IR)
  F-B6  Stage 4        ObservationPlan
  F-B7  Stage 5        RangePlan
  F-B8  Stage 6        StoragePlan ("the bridge")
  F-B9  Stage 7        SramPagePlan
  F-B10 Stage 8        RomWindowPlan
  F-B11 Stage 8.5      OverlayPlan
  F-B12 Stage 9        ArenaPlan
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation         ← THIS RFC
  F-B14 Stage 11       ScheduleCostAnalysis
  F-B15 Stage 12       Backend (AsmIR + ReachabilityValidation +
                                PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs
        (BLOCKED on oracle question)
  F-B17 StageCache integration sweep across all stages
```

Sequencing of weekly chunks:

```text
Chunk 1 (in flight):  F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2 (drafted):    F-B3 + F-B5         Stages 1, 3
Chunk 3 (drafted):    F-B6 + F-B7         Stages 4, 5
Chunk 4 (drafted):    F-B8                Stage 6
Chunk 5 (drafted):    F-B9 + F-B10        Stages 7, 8
Chunk 6 (drafted):    F-B11 + F-B12       Stages 8.5, 9
Chunk 7 (THIS RFC):   F-B13               Stages 10, 10.5
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B13 sits in the pipeline

F-B13 is the **schedule + proof** chunk. It is the most consequential
transform stage: it commits values, effects, and materializations to
**slices**, **leases**, **mutation**, **aliasing**, and **resumable
control flow**. It is also the chunk that produces the **typed proof**
(`ResourceStateValidation`) that every lease is balanced, every yield is
legal, and every ISR-visible path is residency-safe. Multi-mode
scheduling (`SchedulePack` keyed by `RuntimeMode`) is introduced here.

* **Stage 10 — `GbSchedIR`.** Consumes the value/effect IR and every
  storage/range/SRAM/ROM/overlay/arena plan and commits them into
  explicit **slices** (`SchedSlice`) carrying
  `hard_cycles_to_safe_point`, `soft_target_cycles`,
  `max_interrupt_latency`, `ResourceVector`, `ArenaSlot` live-sets,
  `YieldKind`, `YieldCheckClass`, `entry_residency`, `InterruptPolicy`,
  `required_leases`, and `ExitKind`. It commits explicit
  **resource leases** (`ResourceLease` over `RomWindowBinding`,
  `SramPageBinding`, `OverlayId`, `InterruptMask`) at the SchedOp
  granularity. It collects slices and leases into **residency epochs**
  (`ResidencyEpoch`) keyed by `(RomWindowBinding, OverlayId, Residency)`.
  It assembles a multi-mode **`SchedulePack`** keyed by `RuntimeMode`
  (interactive typing, steady-state generation, trace-heavy debugging)
  whose modes share artifact semantics, checkpoint schema, and
  continuation ABI. Switching between modes is declared in
  `ModeSwitchPolicy` and bounded by `RuntimeDriftMonitor` /
  `DriftEnvelope` / `DriftTrigger` / `DriftAction`.

* **Stage 10.5 — `ResourceStateValidation`.** A **typed proof**, not a
  runtime check, run after `GbSchedIR`. It uses lease-flow analysis over
  `SchedSlice` operations and proves:
  1. all resource leases are balanced (every `AcquireLease` has a
     matching `ReleaseLease` on every reachable slice path);
  2. no illegal yield crosses a non-resumable lease;
  3. no ISR-visible path depends on leased switchable state;
  4. overlay and bank-shadow assumptions match the slice's declared
     residency.
  Outputs `certs/resource_state.cert.json`, a machine-checkable
  certificate.

```text
   QuantGraph (F-B3) ─── ArtifactOracle (F-C2)
        |
        v
   GbInferIR (F-B5) ─── ArtifactOracle, op-for-op
        |
        +--> ObservationPlan (F-B6)        ─ semantic_checkpoint pins
        |        |
        |        v
        +--> RangePlan (F-B7)              ─ tile sizes, reduction sites
        |        |
        |        v
        +--> StoragePlan (F-B8) ─ Materialization, LifetimeClass, AliasClassId
                    |
        +--> SramPagePlan (F-B9) ─ SramPageBinding
                    |
        +--> RomWindowPlan (F-B10) ─ RomWindowBinding, KernelResidency
                    |
        +--> OverlayPlan (F-B11) ─ OverlayId, OverlayInstall
                    |
        +--> ArenaPlan (F-B12) ─ ArenaSlot byte ranges
                    |
                    v
        +-----------------------------------+
        | GbSchedIR (Stage 10, F-B13)       |    ← THIS RFC
        |                                   |
        |   slices (SchedSlice)             |
        |   ops    (SchedOp)                |
        |   leases (ResourceLease)          |
        |   epochs (ResidencyEpoch)         |
        |   pack   (SchedulePack[RuntimeMode]) |
        |   policy (ModeSwitchPolicy)       |
        |   drift  (RuntimeDriftMonitor)    |
        |                                   |
        |   emits sched_ir.json             |
        |   emits slice_report.json         |
        +-----------------+-----------------+
                          |
                          v
        +-----------------------------------+
        | ResourceStateValidation           |    ← THIS RFC
        | (Stage 10.5, F-B13)               |
        |                                   |
        |   typed lease-flow analysis       |
        |     (not runtime simulation)      |
        |                                   |
        |   proves:                         |
        |     - lease balance               |
        |     - no illegal yield crossings  |
        |     - ISR-visible paths residency |
        |     - overlay/bank-shadow match   |
        |                                   |
        |   emits certs/resource_state.cert.json
        +-----------------+-----------------+
                          |
                          v
        +-----------------------------------+
        | ScheduleCostAnalysis (F-B14)      |  ─ per-mode cost envelopes
        | Backend incl. ReachabilityValidation (F-B15)
        | FeasibilityRefinementLoop (F-B16) ─ repair proposals
        | Runtime cooperative-kernel scheduler (F-D1)
        +-----------------------------------+
```

### 0.4 Cross-epic interactions

F-B13 sits at the intersection of five epics:

```text
Epic A → Epic B
  - gbf-foundation (Hash256, BlobRef, sized-byte-budget wrappers,
                    typed ids, SemVer wrappers)                    consumed
  - gbf-hw (TargetProfile, MemoryMap regions, MBC5 register
            constants, ISR vector layout)                          consumed
  - gbf-abi (PersistHeader, PersistKind, PageState, CommitGroupId,
             InferenceState liveness fields, HarnessCommandBlock,
             HarnessResultBlock, SemanticCheckpointId,
             CompactCheckpointId, RuntimeFlags layouts)            consumed
  - gbf-runtime::banking (BankLease/BankGuard ABI; lease shapes
             this RFC describes mirror the runtime ABI semantics)  mirrored
  - gbf-runtime::persistence (commit-boundary contract that bounds
             where `Persist` materializations may be written)      consumed
  - gbf-store (StageCache) for K10 / K10.5 cache wiring            consumed
  - gbf-asm (cycle model uses are confined to budget arithmetic;
             actual cycle values come from F-B14, not here)        deferred

Epic B (internal):
  - F-B2 / F-B4 ReportEnvelope rule + StageCache convention        inherited
  - F-B3 / F-B5 IR products (`QuantGraph`, `GbInferIR`,
                              `EffectId`, `ValueId`,
                              `SemanticCheckpointId`)              consumed
  - F-B6 ObservationPlan products (semantic-checkpoint pins,
                                    trace-probe pins, observability
                                    mode + trace-budget caps)      consumed
  - F-B7 RangePlan products (reduction structure, tile sizes,
                              reduction-loop slice boundaries)     consumed
  - F-B8 StoragePlan products (Materialization, LifetimeClass,
                                AliasClassId)                       consumed
  - F-B9 SramPagePlan products (SramPageBinding, page-switch budgets,
                                 spill policy, commit boundaries)  consumed
  - F-B10 RomWindowPlan products (RomWindowBinding, KernelResidency)  consumed
  - F-B11 OverlayPlan products (OverlayId, OverlayInstall,
                                 OverlayLeaseShape, share classes) consumed
  - F-B12 ArenaPlan products (ArenaSlot, NamedArena, byte ranges)  consumed
  - F-B14 ScheduleCostAnalysis                                     feeds
  - F-B15 Backend including ReachabilityValidation                 feeds
  - F-B16 FeasibilityRefinementLoop (BLOCKED on oracle)            feeds
  - F-B17 StageCache cross-cut                                     compatible

Epic C → Epic B (oracle correspondence):
  - F-C3 ScheduleOracle consumes `SchedulePack` and binds emulator
        harness state to slice boundaries; consumes the ABI bounds
        that this RFC pins.                                         provided

Epic D → Epic B (runtime handshake):
  - F-D1 cooperative-kernel scheduler is the runtime side of the
        SchedulePack/ModeSwitchPolicy/DriftEnvelope handshake.
        F-D6 SchedulePack mode switching extends this; both
        consume the contracts pinned here.                          provided

Epic F → Epic B:
  - certs/resource_state.cert.json is a canonical certificate.       produced
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk straddles the front of M3 and
unblocks M3's "value/effect `GbInferIR` + … wired end-to-end for a routed
FFN under the cooperative scheduler" commitment, plus the M2 backend
work that depends on `SchedulePack`:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring.
              ↳ F-B13 is downstream of M1; the M1 quantized dense kernel
                will be scheduled into one Default-mode `SchedulePack`
                fixture so this chunk's gates are exercised end-to-end.

M2            One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ F-B13 is what makes ScheduleOracle implementable: slices
                are the granularity at which ScheduleOracle binds emulator
                state. ReachabilityValidation in F-B15 is the *computed*
                version of the residency-safety claims this RFC checks
                against annotations (see §9.4).

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B13 is the M3 commitment delivery: slices, leases,
                resumable control flow, and the cooperative-kernel
                runtime contract are all in this chunk. Without it,
                the cooperative scheduler has no schedule to host.

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ F-B13 lands the *schema* for SchedulePack mode switching
                (modes, legal switch points, drift triggers); the runtime
                producer/consumer of that schema lives in F-D6
                (BLOCKED on oracle). M4's persistence work consumes the
                `Materialization::Persist` slot lifetimes pinned here.
```

### 0.6 What the project as a whole gains when this chunk lands

```text
1. The schedule is real.
   Before this chunk, the IR says what the program means; after this
   chunk, the schedule says how it actually runs on a cooperative
   kernel with bounded interrupt latency. Every slice has a typed
   contract, not a vibe.

2. Leases are first-class.
   AcquireLease and ReleaseLease become typed SchedOps. Lease-flow
   analysis (§9.3) is what makes "no illegal MBC write on an ISR-reachable
   path" a typed proof rather than a code-review checkpoint.

3. Multi-mode scheduling has a schema.
   SchedulePack keyed by RuntimeMode is in tree. Even before F-D6
   unblocks, the artifact carries one mode (Default) and the schema
   accommodates more without an artifact bump.

4. The drift envelope is observable.
   Compiler produces `expected`; runtime produces `observed`. Runtime
   drift becomes a structured handshake instead of a silent slowdown.
   Drift triggers carry typed actions (ShrinkSlices, DropTrace,
   DemoteMode).

5. ResourceStateValidation closes the cooperative-kernel correctness
   loop.
   Lease balance, ISR safety, yield safety, residency match — these
   are the four properties whose violation produces the worst class of
   bugs (lock-ups twenty minutes into a ROM run). All four are now
   typed-proof obligations.

6. certs/resource_state.cert.json is canonical.
   F-F2's certificate set gains a third member (after range and arena);
   reachability arrives in F-B15. The certificate is independently
   checkable and pins every load-bearing decision.

7. F-B14 (cost) and F-B15 (backend) become implementable.
   F-B14 charges cycles against per-slice budgets; F-B15 emits AsmIR
   sections that match slice boundaries. Without F-B13 there is nothing
   to charge against and no slice boundary to emit at.

8. F-C3 (ScheduleOracle) becomes implementable.
   ScheduleOracle binds emulator state to slice boundaries. This RFC
   pins those boundaries.

9. Runtime is the producer of "observed", compiler is producer of
   "expected".
   The runtime side ABI (cooperative scheduler, drift monitor) is
   contracted to consume `SchedulePack` verbatim. F-D1 wires the runtime;
   this RFC owns the schema.
```

### 0.7 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later stage receives a typed `SchedulePack` whose self-hash pins
  every load-bearing scalar (slice ids, lease ids, mode set, switch
  policy, drift envelope, checkpoint schema hash). F-B14 never invents a
  slice; F-B15 never invents a section; F-C3 never invents a checkpoint
  pin.
* `ResourceLease`, `LeaseId`, `EpochId`, `SliceId`, `ResidencyEpoch`,
  `ResourceVector`, `YieldKind`, `YieldCheckClass`, `ExitKind`,
  `InterruptPolicy`, `RuntimeMode`, `ModeSwitchPolicy`,
  `DriftEnvelope`, `DriftTrigger`, `DriftAction` names are stable across
  runs. Symbol map, listing, and `.sym` output (Epic A) consume them.
* The lease-balance and yield-safety obligations are discharged at
  Stage 10.5 *before* `AsmIR` exists. F-B15 must still prove
  *computed* reachability (ISR-visible code/data is Bank0/HRAM/fixed-WRAM
  only); this RFC's annotation-driven check is the upper bound on which
  Stage 10.5 fails fast (see §9.4).
* The runtime side ABI is contracted: cooperative-kernel scheduler and
  drift monitor have a typed schema to consume; F-D1 wires it.

### 0.8 Reading order for reviewers

```text
§0  (this section) — placement and dependencies
§0a TL;DR
§1  Project context — milestone-specific framing
§2  Load-bearing decisions — the engineering choices that bracket the rest
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 10 and Stage 10.5 plug in
§8  Stage 10 contract: GbSchedIR
§9  Stage 10.5 contract: ResourceStateValidation
§10 SchedulePack multi-mode semantics
§11 Drift monitor contract
§12 Liveness contract
§13 Report schemas
§17 Task DAG
§19 Proof obligations
§20 End-to-end theorem
§21 Final concise contract
```

Skim §3, §4, §7, §14, §15, §16, §18 for specifics.

## 0a. TL;DR

This chunk lands the **schedule + proof** pair. It owns two numbered
stages whose coupling is intentionally tight — Stage 10 produces the
schedule, Stage 10.5 proves the schedule is interrupt-safe,
lease-balanced, and residency-correct. They are paired in one Feature
because the proof's input *is* the schedule's output, and the proof
delivers a separate certificate (`certs/resource_state.cert.json`)
distinct from the schedule's report (`sched_ir.json`).

* **Stage 10 — `GbSchedIR`.** Commits the value/effect IR
  (`GbInferIR`, F-B5), the observation/range/storage/sram-page/rom-window/
  overlay/arena plans (F-B6 → F-B12), and the resolved compile policy
  (F-B2) into:
  * **slices** (`SchedSlice`) carrying `hard_cycles_to_safe_point`,
    `soft_target_cycles`, `max_interrupt_latency`, `ResourceVector`,
    `live_wram` / `live_sram` (over `ArenaSlot`), `YieldKind`,
    `YieldCheckClass`, `entry_residency`, `InterruptPolicy`,
    `required_leases`, `ExitKind`;
  * **resource leases** (`ResourceLease`) over `RomWindowBinding`,
    `SramPageBinding`, `OverlayId`, `InterruptMask`;
  * **residency epochs** (`ResidencyEpoch`) keyed by
    `(RomWindowBinding, OverlayId, Residency)`;
  * **`SchedulePack`** keyed by `RuntimeMode` (`InteractiveTyping`,
    `SteadyStateGeneration`, `TraceHeavyDebugging`) with a single
    artifact-shared `checkpoint_schema_hash` and continuation ABI;
  * **`ModeSwitchPolicy`** (`legal_switch_points`,
    `legal_epoch_boundaries`, `ui_pressure_thresholds`,
    `safe_mode_triggers`, `drift_triggers`);
  * **`RuntimeDriftMonitor`** with paired `DriftEnvelope`s (compiler-side
    `expected`, runtime-side `observed`), `DriftTrigger` thresholds, and
    `DriftAction` responses (`ShrinkSlices`, `DropTrace`, `DemoteMode`).

* **Stage 10.5 — `ResourceStateValidation`.** A typed proof, not a runtime
  check. Runs lease-flow analysis over `SchedSlice` operations and
  proves:
  1. **lease balance:** every `AcquireLease` has a matching `ReleaseLease`
     on every path through the slice graph reachable from any slice
     entry;
  2. **yield-safety:** no `Yield`-class `SchedOp` transition crosses a
     non-resumable lease (i.e. a lease whose `yield_safe = false`);
  3. **ISR-visible residency:** no slice marked
     `interrupt_policy = Enabled` (or any path thereto) depends on leased
     switchable state — i.e. a `ResourceLease` whose
     `kind ∈ { RomWindow, SramPage }` is held while interrupts are
     enabled and ISR-reachable code touches the leased region;
  4. **overlay/bank-shadow consistency:** every slice's
     `entry_residency` matches the `ResidencyEpoch` it belongs to, and
     every `OverlayInstall` referenced by a lease is a member of the
     declared `ResidencyEpoch`'s overlay set.
  Emits `certs/resource_state.cert.json` (machine-checkable). The
  certificate is independently re-runnable by `gbf-verify`.

These two stages are paired in one Feature (F-B13) because:

(a) Stage 10.5 has exactly one input — the Stage 10 product — and emits
exactly one artifact (the certificate). Splitting them would force a
synthetic boundary that no other consumer respects.

(b) Stage 10 alone does not close: every project gate that consumes
`SchedulePack` (F-B14, F-B15, F-C3, F-D1) requires
`certs/resource_state.cert.json` to exist before it accepts the pack.
The two stages are the smallest unit that makes downstream consumers
honest.

(c) The shared diagnostic envelope, JSON canonicalization rule, self-hash
convention, and StageCache key construction inherited from F-B2/F-B4
treat both stages identically. Splitting them would duplicate boilerplate
without a contract benefit.

The chunk closes only when:

1. `GbSchedIR` construction is a deterministic pure function of the
   pinned upstream products (`GbInferIR`, `ObservationPlan`,
   `RangePlan`, `StoragePlan`, `SramPagePlan`, `RomWindowPlan`,
   `OverlayPlan`, `ArenaPlan`, `ResolvedCompilePolicy`,
   `RuntimeChromeBudget`) and is byte-identical across two consecutive
   regenerations on a clean checkout.

2. `ResourceStateValidation` is a deterministic pure function of
   `GbSchedIR` and is byte-identical across two consecutive regenerations
   on a clean checkout.

3. `sched_ir.json`, `slice_report.json`, and
   `certs/resource_state.cert.json` round-trip through their semantic
   validators and self-hashes.

4. Every `SchedSlice` has a finite `hard_cycles_to_safe_point` no smaller
   than its `max_interrupt_latency`. Every slice's `required_leases` is a
   subset of the leases acquired in scope. Every slice's `live_wram` and
   `live_sram` are `ArenaSlot`s whose `LifetimeClass` covers the slice's
   lifetime. Every slice's `entry_residency` is consistent with the
   `ResidencyEpoch` it belongs to.

5. Every `ResourceLease` is acquired and released exactly once on every
   reachable slice path. Every yield-class transition that crosses a
   lease has `yield_safe = true` for that lease. Every slice with
   `interrupt_policy = Enabled` proves (via annotated reachability,
   pending F-B15 ReachabilityValidation) that no leased switchable state
   is reachable from any ISR vector through any slice path.

6. `SchedulePack` carries at least one mode (`Default` for v1) and any
   additional modes share `checkpoint_schema_hash` and continuation ABI
   with the default mode. `ModeSwitchPolicy.legal_switch_points` is a
   subset of the artifact's `SemanticCheckpointId` set; switches at any
   other point are typed rejects.

7. `RuntimeDriftMonitor` carries a non-default `DriftEnvelope` whose
   `slice_cycles_p95` field is `Some` (other fields may be `None` until
   the runtime measurement bus is wired); every `DriftTrigger.action`
   exists in the closed `DriftAction` set; every `DriftTrigger.metric`
   exists in the closed `DriftMetric` set.

8. `StageCache` keys K10 (Stage 10) and K10.5 (Stage 10.5) are pinned
   and tested.

The chunk does **not** include:

* Codegen — owned by F-B15 (Stage 12). F-B13 declares slice contracts;
  F-B15 emits the bytes that satisfy them.
* Register allocation — owned by F-B15.
* Far-call legalization — owned by F-B15. F-B13 records call shapes and
  bank-switch counts; F-B15 inserts the trampolines.
* Byte placement — owned by F-B12 (already done) and F-B15 (sections).
* Cycle-cost production — owned by F-B14 (`ScheduleCostAnalysis`).
  F-B13 declares slice budgets in cycles; F-B14 charges actual cycles
  against them under the calibration bundle.
* `ReachabilityValidation` — owned by F-B15. F-B13's Stage 10.5 checks
  declared residency against ISR rules; F-B15's `ReachabilityValidation`
  is the *computed* version (transitive reachability after far-call
  legalization).
* Refinement-loop repairs — owned by F-B16 (BLOCKED). F-B13 leaves
  `RepairProposal` pluggable in the data layout; no proposal is consumed
  here.
* Trace data production — owned by F-B14 / Epic D.
* Persistence producer/consumer — owned by Epic D.
* Runtime drift measurement — owned by F-D1 (cooperative-kernel
  scheduler). F-B13 declares `DriftEnvelope` shape and thresholds;
  F-D1 emits the observed envelope.
* SchedulePack mode-switching execution — owned by F-D6 (BLOCKED on
  oracle). F-B13 pins the schema; F-D6 wires the runtime side.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2 / F-B3 / F-B4 / F-B5 / F-B6 / F-B7 / F-B8 / F-B9 / F-B10 / F-B11 / F-B12 leave on the table

By the time F-B13 begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, calibration, hint bundle, and
  `CompileRequest` are admissible and hash-bound through
  `artifact_validation.json` (F-B2 Stage 0).
* `ResolvedCompilePolicy` is the single answer to "what policy governed
  this build" with provenance for every load-bearing scalar (F-B2 Stage
  0.5). `requested_runtime_modes` is part of the resolved policy.
* `RuntimeChromeBudget` has been honored at the static byte-math level
  (F-B4 Stage 2) with a successful `static_budget.json`, including
  projected bank-switch and SRAM-page-switch counts per token.
* `QuantGraph` (F-B3 Stage 1) and `GbInferIR` (F-B5 Stage 3) are
  content-addressed and storage-free. `GbInferIR` carries explicit
  `ValueId` / `EffectId` edges for sequence-state mutation and RNG
  progression.
* `ObservationPlan` (F-B6 Stage 4) has bound the
  `SemanticCheckpointId` and `TraceProbeId` pin set, the
  `ObservabilityMode`, and the `TraceBudget`.
* `RangePlan` (F-B7 Stage 5) has bound the reduction structure and
  reduction-loop tile sizes; reduction-site identity is hashed.
* `StoragePlan` (F-B8 Stage 6) has decided, for every `ValueId` in
  `GbInferIR`, whether the value is `Recompute`,
  `Materialize { class, lifetime }`, or
  `Persist { page, commit_group }`, plus its `AliasClassId`.
* `SramPagePlan` (F-B9 Stage 7) has assigned page-state geometry, page
  rotation, spill policy, and commit boundaries to every `Persist`
  binding whose `class` is `SramPaged`. `SramPageBinding` records
  page-switch budgets per token.
* `RomWindowPlan` (F-B10 Stage 8) has resolved kernel and LUT residency:
  `Bank0Fixed`, `WramOverlay`, or `CoResidentSwitchable`. `RomWindowBinding`
  records the simultaneously-visible ROM set.
* `OverlayPlan` (F-B11 Stage 8.5) has assigned `OverlayId`s,
  `OverlayShareClass`es, and `OverlayInstall` events with static
  `OverlayLeaseShape` descriptors.
* `ArenaPlan` (F-B12 Stage 9) has assigned named arenas
  (`NamedArena`) and concrete byte ranges to every materialized value;
  `ArenaSlot` records preserve `LifetimeClass` and `AliasClassId` from
  `StoragePlan`. Persistent pages match the SRAM persistence protocol.

What is *not* yet decided when this chunk begins:

* No object has been split into slices. Compute is in `GbInferIR` ops;
  the cooperative-kernel boundary at which yields are legal is not yet
  declared.
* No lease lifecycle exists. `OverlayInstall.lease_shape` is a static
  descriptor; F-B13 mints `LeaseId`s and pairs `Acquire` / `Release`
  events.
* No interrupt-latency budget per slice is in tree.
* No multi-mode scheduling (`SchedulePack` keyed by `RuntimeMode`)
  exists.
* No drift envelope has been bound to compiled choices.
* No proof of lease balance, yield safety, or ISR-visible residency
  exists; without `certs/resource_state.cert.json`, downstream consumers
  cannot accept the schedule.

This chunk is responsible for closing those gaps deterministically and
auditably.

### 1.2 Why these two stages are paired

The natural unit is "the single transform that commits values, effects,
and materializations to slices, leases, mutation, aliasing, and resumable
control flow — together with the typed proof that the commitment is
sound."

* If we made it one stage (just Stage 10), the proof obligations would
  blur with the construction. Stage 10's job is to *produce* a candidate
  schedule; lease-balance and yield-safety obligations live in a
  *separate* analysis. Folding them into Stage 10 would make the schedule
  itself untrustable without re-running the analysis.
* If we made it three stages (e.g. Stage 10 sched, Stage 10.5
  lease-flow, Stage 10.7 residency proof), we would split on internal
  structure that re-converges at certificate emission. Lease-flow,
  yield-safety, and ISR-visible-residency are three properties that share
  one input (the slice graph) and emit one certificate. Splitting them
  would fragment the certificate and introduce three intermediate cache
  keys without a contract benefit.
* Two stages — Stage 10 producing the schedule, Stage 10.5 producing the
  certificate — is the natural seam. The seam is *typed*: Stage 10's
  output is the typed `SchedulePack`, Stage 10.5's input is exactly that
  pack, Stage 10.5's output is the certificate alongside an unchanged
  pack reference.

The pairing is also pragmatic: every consumer of `SchedulePack` (F-B14,
F-B15, F-C3, F-D1) requires both the pack and the certificate. A
two-stage chunk lets the certificate be content-addressed against the
pack hash, so downstream consumers can hash-verify both with one
StageCache hit.

### 1.3 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later stage receives a typed `SchedulePackProduct` and a typed
  `ResourceStateCertificate` whose self-hashes pin every load-bearing
  scalar.
* F-B14 (`ScheduleCostAnalysis`) consumes `SchedulePack` and produces
  per-mode cost envelopes against the calibration bundle. F-B13 owns
  the slice contracts; F-B14 owns the cycle math.
* F-B15 (Backend) emits sections that match slice boundaries, far-call
  edges that respect lease shapes, and reachability classes that
  *compute* what F-B13 *annotated*. The handshake is:
  F-B13 declares `interrupt_policy`, `entry_residency`, and
  `required_leases`; F-B15 verifies that the *computed* transitive
  reachability of every ISR vector contains only Bank0/HRAM/fixed-WRAM
  code, every yield-resume target is reachable, and every fault path is
  residency-honest.
* F-C3 (`ScheduleOracle`) binds emulator harness state to slice
  boundaries; the slice contract is fixed here.
* F-D1 (cooperative-kernel scheduler) consumes `SchedulePack` verbatim;
  the runtime side of `RuntimeDriftMonitor` produces `observed`
  envelopes that are compared against the `expected` envelope this RFC
  pins.

### 1.4 Why this is one Feature, not two

The tight pairing of Stage 10 and Stage 10.5 makes a single Feature
the right granularity:

* The certificate's input is exactly the Stage 10 product. There is no
  external surface between them — no other consumer exists between
  Stage 10 and Stage 10.5 — so no PR cuts cleanly between them.
* Both stages share inputs by reference (the upstream plans) and outputs
  by hash; one StageCache hit covers both products.
* The reviewer surface is one feature bead (bd-9ae) with one PR shape;
  splitting into two beads would force a synthetic dependency edge whose
  only consumer is the certificate, and would fragment the proof
  obligations across two reviews.

That said: the chunk RFC treats the two stages as separate **contracts**
(§8 vs §9). They have different schemas, different StageCache keys
(K10 and K10.5), and different rejection classes. Implementations may
share helpers but must keep the contracts typed.

### 1.5 What this chunk is NOT

The chunk is large in scope and very large in contract surface. To
prevent scope creep:

* It is **not** codegen. F-B15 emits AsmIR; F-B13 declares slice
  contracts. No byte sequence, no `Db` directive, no opcode mnemonic
  appears in `SchedSlice` or `SchedOp`.
* It is **not** register allocation. F-B15 does ISA-aware register
  selection inside slices. F-B13's `SchedOp` is hardware-aware but
  register-free.
* It is **not** far-call legalization. F-B13 records call shapes and
  cross-bank dependencies; F-B15 inserts the trampolines and rewrites
  branches.
* It is **not** byte placement. F-B12 placed materialized values; F-B15
  places sections. F-B13 reads `ArenaSlot` byte ranges but does not
  move bytes.
* It is **not** a cycle-cost producer. F-B13's cycle fields
  (`hard_cycles_to_safe_point`, `soft_target_cycles`,
  `max_interrupt_latency`) are budgets — bounds the schedule must
  respect. The actual cycle counts are produced by F-B14's
  `ScheduleCostAnalysis` against the calibration bundle. F-B13 does
  *not* consume calibration.
* It is **not** a runtime drift monitor. F-B13 declares
  `DriftEnvelope.expected`; the runtime (F-D1) produces
  `DriftEnvelope.observed`. The handshake is in the schema; the runtime
  side is in Epic D.
* It is **not** a fault-policy recovery exerciser. Fault paths declare
  residency (§8.4); F-B13 checks the declaration against ISR rules.
  Recovery exercise lives in Epic D.
* It is **not** a refinement loop. `RepairProposal` and `KnobDelta` are
  plumbed as named-only hooks; no repair is consumed here. F-B16 is
  BLOCKED on oracle.
* It is **not** an autoregressive driver. `SchedulePack` represents the
  compute for a single-token pass under a chosen `RuntimeMode`; the
  multi-token loop is at runtime.
* It is **not** an emulator integration. Slice contracts are pure
  functions of typed inputs; no emulator run is required to test the
  chunk. F-C3's emulator integration consumes `SchedulePack` separately.
* It is **not** the producer of the harness command/result blocks. The
  harness blocks are arena slots (F-B12); F-B13 references them through
  `live_wram` or `live_sram` membership when a slice exposes a harness
  result.
* It does **not** mutate the upstream products. `SchedulePack` carries
  references to `ArenaSlot`, `OverlayId`, `RomWindowBinding`,
  `SramPageBinding`, `Materialization`, `LifetimeClass`, `AliasClassId`,
  `SemanticCheckpointId`, `ValueId`, and `EffectId` by hash; it does not
  redefine them.

### 1.6 Relationship to F-A4 BankLease/BankGuard ABI

F-A4 (`gbf-runtime::banking`) owns the runtime ABI for ROM-bank writes,
SRAM-bank writes, and RAM enable/disable. The legal path to MBC writes
is exactly the `BankLease` / `BankGuard` token discipline. F-B13's
`ResourceLease` is the **compile-time mirror** of that runtime ABI:

```text
runtime side:   BankLease / BankGuard (a token discipline that brackets
                hardware writes with shadow updates and a short critical
                section).

compile side:   ResourceLease (a typed acquire/release pair on
                `ResourceLeaseKind ∈ { RomWindow, SramPage, Overlay,
                                       InterruptMask }`).
```

The two are isomorphic by construction:

* every compile-time `ResourceLease::Acquire` lowers (in F-B15) to a
  runtime `BankLease::acquire`-shaped call (or its overlay /
  interrupt-mask analog);
* every compile-time `ResourceLease::Release` lowers to the matching
  runtime release;
* every compile-time `yield_safe = false` lease implies the runtime guard
  bracket is held across no yield boundary;
* every compile-time `interrupt_policy = ShortCriticalSection` slice
  matches the runtime's "interrupts-disabled scope around a hardware
  write" pattern.

F-A4's lease ABI is the source of truth for *runtime* behavior; F-B13
is the source of truth for *compile-time* lease shape. F-B15 is the
adapter that emits the runtime calls satisfying the compile-time shape.

### 1.7 Relationship to F-B15 ReachabilityValidation

F-B15's `ReachabilityValidation` is the **computed** version of the
residency-safety claims this RFC checks against **annotations**. The
distinction matters:

* F-B13's Stage 10.5 trusts the slice's declared `entry_residency`,
  `interrupt_policy`, and `required_leases`. It checks that the
  declarations are *internally consistent* and *consistent with the
  upstream plans* (`KernelResidency`, `RomWindowBinding`,
  `OverlayInstall`, `ArenaSlot`). It does *not* compute transitive
  reachability through far-call edges, because far-call legalization
  has not happened yet.
* F-B15's `ReachabilityValidation` is the whole-program reachability
  pass. After far-call legalization and thunk insertion, it computes
  `ISR-reachable`, `yield-resume reachable`, `fault-path reachable`,
  `harness-entry reachable`, `bank-lease protected`, and `normal only`
  classes, and validates them against the same residency rules.

In effect:

```text
F-B13 (Stage 10.5)  ─ "the declarations are honest at the schedule
                       level; lease-flow is balanced; yield safety is
                       local; fault-path declarations match
                       residency."

F-B15 (Reachability) ─ "the declarations survive far-call legalization
                         and global section ordering."
```

The order of operations is: F-B13 first (declarations are locally
honest), F-B15 last (declarations are globally honest). If F-B15
discovers a violation, F-B16 (BLOCKED) is the loop that may shrink
slices, demote trace, or promote overlays to repair. F-B13 leaves the
hooks in place (§16) without consuming them.

### 1.8 Relationship to F-D1 cooperative-kernel scheduler

F-D1 is the runtime side of the SchedulePack contract. The runtime:

* loads `SchedulePack` at boot (or recovers it from
  `gbf-abi::InferenceState` after a yield);
* selects the active `RuntimeMode` based on UI pressure, drift
  triggers, and explicit user/harness commands (`SafeMode` defaults
  apply);
* enters slices through `gbf-abi::cont_*` continuation fields; saves
  continuation state at slice boundaries;
* observes per-slice cycles, yield latencies, trace-drop rates, and
  persist-overrun rates against the `DriftEnvelope.expected` envelope
  this RFC pins;
* triggers `DriftAction`s (`ShrinkSlices`, `DropTrace`, `DemoteMode`)
  per the `ModeSwitchPolicy.drift_triggers` thresholds.

F-D1 does not modify `SchedulePack`. The runtime is the producer of
`observed`, the consumer of `expected`. F-B13 owns `expected`; F-D1
owns `observed`.

## 2. Load-bearing decisions

### 2.1 Pure-function shape (core / driver split)

Both stages have **two layers**: a pure core constructor and a thin
driver that performs IO. The core is a pure function from typed pinned
inputs to typed content-addressed products. The driver wraps the core
with JSON / certificate emission and StageCache writes.

```text
build_sched_ir_core(SchedIrInputs)
  -> Result<(SchedulePack, ReportEnvelope<SchedIrReportBody>,
             ReportEnvelope<SliceReportBody>),
            PassDiagnostics>

run_stage10(SchedIrInputs, env)
  = build_sched_ir_core(...) then
    (on success or failure):
      emit sched_ir.json
      emit slice_report.json
      may write StageCache success entry (K10)
      may write StageCache failure memo

build_resource_state_cert_core(ResourceStateInputs)
  -> Result<(ResourceStateCertificate,
             ReportEnvelope<ResourceStateCertBody>),
            PassDiagnostics>

run_stage10_5(ResourceStateInputs, env)
  = build_resource_state_cert_core(...) then
    (on success or failure):
      emit certs/resource_state.cert.json
      may write StageCache success entry (K10.5)
      may write StageCache failure memo
```

Cores never mutate the upstream products. Drivers are the only IO
surface. Determinism is required, not aspirational.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed residency-epoch construction)
       (typed slice formation)
       (typed lease binding)
       (typed checkpoint pinning)
       (typed mode-pack assembly)
       (typed drift-envelope binding)
       (Stage 10.5: typed lease-flow analysis)
       (Stage 10.5: typed yield-safety analysis)
       (Stage 10.5: typed residency-consistency check)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed product, ReportEnvelope<ReportV1> }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON
       emits cert (Stage 10.5 always)
       writes StageCache success / failure memo
```

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.

### 2.2 Inheritance from F-B2/F-B4, F-B3/F-B5, and F-B11/F-B12

This RFC inherits, **unchanged**, the following:

* `ReportEnvelope<R>` shape and public JSON conventions — F-B2/F-B4 §4.
* `Hash256`, `DomainHash(...)`, `SelfHash(report)`, `ZERO_HASH` — F-B2/F-B4 §1.
* `CanonicalJson(x)` rule (UTF-8, lex object keys, integers only, no
  NaN/Inf, no unknown fields, explicit enum tags, deterministic array
  ordering where order is not semantically meaningful) — F-B2/F-B4 §2.5.
* `null` policy (only for explicit semantic absence; never for unknown,
  unmeasured, or omitted) — F-B2/F-B4 §2.5.
* `R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`, `R-FlatEnvelope`,
  `R-UnknownReject`, `R-HardOnly-ThisChunk` envelope laws —
  F-B2/F-B4 §4.
* `ValidationDiagnostic` shape (`severity`, `origin`, `code`, `detail`,
  `provenance`) — F-B2/F-B4 §5. New origins and codes are introduced in
  §15 of this RFC; they extend the closed enum without modifying
  existing variants.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §5.
* `R-NoPartialProduct`: failed reports have `body.result = None`. F-B13
  reports MUST NOT contain a partial `SchedulePack` or partial
  certificate — F-B3/F-B5 §7.
* StageCache key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
  — F-B2/F-B4 §11.
* StageCache success/failure-memo cache laws — F-B2/F-B4 §2.6.
* `ArenaSlot`, `NamedArena`, `ArenaId` shapes — F-B11/F-B12 §2.4 and
  §2.5. F-B13 references arenas by id verbatim and never carves new
  arenas.
* `OverlayId`, `OverlayInstall`, `OverlayLeaseShape`, `OverlayShareClass`,
  `OverlayInstallEvent` shapes — F-B11/F-B12 §2.3. F-B13 mints concrete
  `LeaseId`s against `OverlayLeaseShape`s.
* `RomWindowBinding`, `KernelResidency`, `Residency` shapes — F-B10.
* `SramPageBinding` shape — F-B9.
* `Materialization`, `LifetimeClass`, `StorageClass`, `AliasClassId`
  shapes — F-B8.
* `SemanticCheckpointId`, `TraceProbeId` shapes — F-B6, F-A3
  (`gbf-abi::checkpoint`).

If a later amendment to F-B2/F-B4, F-B3/F-B5, F-B11/F-B12, or any of the
forthcoming F-B6 → F-B10 RFCs changes any of the above, that amendment
must explicitly amend this RFC by name.

This RFC adds the following to that surface:

* New `ValidationOrigin` variants: `SchedIrConstruction`,
  `ResourceStateValidation`, `ScheduleModePack`, `RuntimeDriftMonitor`.
* New `ReportSchemaId` variants: `sched_ir.v1`, `slice_report.v1`,
  `resource_state.cert.v1`.
* New public product types: `SchedulePack`, `GbSchedIR`, `SchedSlice`,
  `SchedOp`, `ResourceLease`, `ResourceLeaseKind`, `ResourceVector`,
  `ResidencyEpoch`, `ModeSwitchPolicy`, `RuntimeDriftMonitor`,
  `DriftEnvelope`, `DriftTrigger`, `DriftAction`, `DriftMetric`,
  `YieldKind`, `YieldCheckClass`, `ExitKind`, `InterruptPolicy`,
  `RuntimeMode`, `LeaseId`, `EpochId`, `SliceId`, `CycleBudget`,
  `ResourceStateCertificate`.
* New public report bodies: `SchedIrReportBody`, `SliceReportBody`,
  `ResourceStateCertBody`.
* New `StageCacheKey` schemas: `K10 := SchedIrCacheKey`,
  `K10.5 := ResourceStateCacheKey`.

### 2.3 Slice as cooperative-kernel scheduling unit

A **slice** (`SchedSlice`) is the unit at which the cooperative kernel
yields. Every slice carries:

* a unique `SliceId` (stable across runs, content-addressed);
* a sequence of `SchedOp`s with explicit acquire/release operations,
  load/store operations, tile-loop operations, and a terminating
  yield-or-exit operation;
* a **hard cycle budget** to the next safe point
  (`hard_cycles_to_safe_point`) — the worst-case bound on how many
  cycles execution may take before reaching a yield-legal point. This is
  the cooperative-kernel analog of an interrupt-latency ceiling;
* a **soft target cycle count** (`soft_target_cycles`) — the
  compile-time prediction of typical per-slice cycles, used by F-B14
  for cost analysis;
* a **maximum interrupt latency** (`max_interrupt_latency`) — the worst
  case bound on cycles between an ISR firing and ISR entry. For
  `interrupt_policy = Enabled` slices, this is bounded above by hardware
  (LR35902 has a fixed interrupt latency in cycles for a clean opcode
  boundary plus the dispatch table); for `ShortCriticalSection` slices,
  this is the bound on the critical section length; for `Disabled`
  slices, this is the bound on cycles until interrupts are re-enabled;
* a `ResourceVector` summarizing per-slice resource activity
  (bank_switches, sram_page_switches, trace_bytes, persist_bytes,
  overlay_installs);
* `live_wram` and `live_sram` — the set of `ArenaSlot`s that must be
  alive across the slice boundary on entry. These are the slot
  references whose `LifetimeClass` covers the slice's lifetime;
* a `YieldKind` (`Micro`, `Frame`, `NeedInput`, `TokenReady`, `Finished`,
  `Fault`) declaring why this slice ends (or whether it terminates the
  build);
* a `YieldCheckClass` describing how the runtime polls the
  `yield_requested` flag (e.g. once-at-end, every-N-tiles,
  every-load-store-pair);
* `entry_residency` — the `Residency` (Bank0, Common, Expert, etc.) the
  slice expects on entry. This must match the `ResidencyEpoch` the
  slice belongs to;
* `interrupt_policy` (`Enabled`, `ShortCriticalSection`, `Disabled`) —
  the slice's interrupt policy. Most slices are `Enabled`; only
  bracket-around-MBC-write paths use `ShortCriticalSection`; only the
  smallest critical sections (e.g. shadow-register update) use
  `Disabled`;
* `required_leases` — the set of `LeaseId`s the slice depends on at
  entry. Every slice path through the slice graph must acquire these
  leases before entering the slice and release them only after exit;
* `ExitKind` — what the slice does at exit (`SaveContinuationAndYield`,
  `TailCall`, `EnterIsr`, `Halt`, `Fault`).

**Why slice-as-unit:** the alternative would be to make the entire
inference pass a monolithic call. That is operationally fragile on a
4 MHz CPU with VBlank every 1.1 ms: a tile-loop that takes 30 ms is a
hung UI. Slices make yielding a compiler feature, not a runtime hope
(`planv0.md` line 127).

**Why cycle budgets are part of the slice contract:** F-B14 charges
*actual* cycles against the calibration bundle; F-B13 declares *bounds*.
The bound is the schedule's contract with the runtime: "this slice will
not exceed these cycles before yielding."

Amends planv0: `planv0.md` line 1801 lists `hard_cycles_to_safe_point`
and `max_interrupt_latency` but is silent on `soft_target_cycles`. This
RFC pins the field as a compile-time prediction of typical per-slice
cycles. Bounds are hard; targets are soft. F-B14 consumes both.

### 2.4 Lease as first-class resource handle

A **resource lease** (`ResourceLease`) is a typed acquire/release pair on
one of four kinds:

```rust
pub enum ResourceLeaseKind {
    RomWindow(RomWindowBinding),
    SramPage(SramPageBinding),
    Overlay(OverlayId),
    InterruptMask(InterruptPolicy),
}
```

The four kinds correspond to the four classes of switchable runtime
state:

* **`RomWindow`** — the single 16 KiB switchable ROM window at
  `$4000-$7FFF`. Holding this lease implies the runtime has issued a
  `BankLease::acquire` against the lease's bank id; the bank shadow
  register matches the hardware register; and no ISR may depend on the
  current selection.
* **`SramPage`** — the 8 KiB external RAM window at `$A000-$BFFF`.
  Same discipline, different register.
* **`Overlay`** — a WRAM overlay region. The `OverlayId` matches an
  `OverlayPlan.regions` entry. Holding this lease implies an
  `OverlayInstall` has been performed (either previously, on the same
  share class, or as part of this slice).
* **`InterruptMask`** — interrupts disabled or constrained to a short
  critical section. Holding this lease implies `interrupt_policy ∈ {
  Disabled, ShortCriticalSection }`.

Every `ResourceLease` carries:

* a unique `LeaseId` (stable across runs, content-addressed);
* a `kind` (one of the four above);
* `acquired_in: SliceId` — the slice that performs the
  `AcquireLease` SchedOp;
* `released_in: SliceId` — the slice that performs the matching
  `ReleaseLease` SchedOp;
* `yield_safe: bool` — whether yielding (`YieldKind::*`) is legal while
  holding this lease.

**Why first-class lease handles:** the alternative — implicit bank
switches scattered through generated code — is exactly the failure mode
F-A4 was designed to prevent. Making `Acquire` and `Release` typed
SchedOps means lease balance is a typed proof obligation (§9.1), not a
code-review checkpoint.

**Why `yield_safe` is a per-lease scalar:** different lease kinds have
different yield semantics. Holding a `RomWindow` lease across a yield is
illegal (the runtime will switch banks during the yielded period and
the resumed code's bank assumption is stale). Holding an `Overlay` lease
across a yield is legal *if* the overlay region's eviction policy allows
the same member to remain resident across the yield (the share class's
`EvictionPolicy` decides). Holding an `InterruptMask` lease across a
yield is illegal (yield re-enables interrupts).

Amends planv0: `planv0.md` line 1784 defines `ResourceLease` with
`yield_safe: bool` but is silent on the cross-product of kind × yield
semantics. This RFC pins the table in §8.4.

### 2.5 Residency epoch as the granularity at which kernel/data residency holds invariant

A **residency epoch** (`ResidencyEpoch`) is a contiguous run of slices
that share:

* the same `RomWindowBinding` (the same simultaneously-visible ROM set);
* the same optional `OverlayId` (the same WRAM-overlay member resident);
* the same `Residency` (Bank0, Common, Expert).

Within an epoch, the assumption "what is in WRAM and what is in ROM at
$4000-$7FFF" is invariant. Crossing an epoch boundary requires either:

* a `RomWindow` lease release+acquire (bank switch);
* an `OverlayInstall` (overlay member change);
* both (residency-class change).

**Why epochs:** F-B14 charges per-token bank-switch and SRAM-page-switch
counts against budgets. The clean granularity for those counts is
"epoch boundaries crossed per token," not "leases acquired per slice"
(which over-counts) or "slices executed per token" (which under-counts).
Epochs also make `ResourceStateValidation`'s overlay/bank-shadow
consistency check (§9.1.4) a typed equality between
`SchedSlice.entry_residency` and `ResidencyEpoch.residency`.

Amends planv0: `planv0.md` line 1832 defines `ResidencyEpoch` with
fields `id`, `rom_window`, `overlay`, `residency`, `slices`. This RFC
pins the cardinality: a slice belongs to *exactly one* epoch; epochs do
not overlap; the union of all epochs' slice sets is exactly the slice
set of the schedule.

### 2.6 SchedulePack multi-mode shape

A `CompiledBuild` may carry one or more `SchedulePack` modes keyed by
`RuntimeMode`. Modes share artifact semantics, checkpoint schema, and
continuation ABI; they differ in tile sizes, yield spacing, kernel
residency, common-bank pressure, and trace density.

The closed v1 `RuntimeMode` set is:

```rust
pub enum RuntimeMode {
    InteractiveTyping,
    SteadyStateGeneration,
    TraceHeavyDebugging,
    SafeMode,
}
```

* **`InteractiveTyping`** — small slices, frequent yields, tight UI
  pressure thresholds. Optimized for low time-to-first-token and low
  frame jitter.
* **`SteadyStateGeneration`** — larger slices, less-frequent yields,
  steady throughput. Optimized for tokens-per-second under sustained
  generation.
* **`TraceHeavyDebugging`** — small slices, dense trace probes, all
  operational probes enabled. Optimized for diagnostic detail; cycle
  budgets relaxed.
* **`SafeMode`** — minimum-feature mode. Used after a fault or when
  drift triggers demote the mode. All optional features (overlay,
  trace, persist) are disabled; only the `Default`-equivalent core is
  retained.

The runtime may switch between modes only at declared safe boundaries
(`ModeSwitchPolicy.legal_switch_points`). Switches are bounded by:

* `ui_pressure_thresholds` — UI pressure metrics (frame jitter,
  video-commit latency) that trigger a switch from `SteadyStateGeneration`
  to `InteractiveTyping` (or to `SafeMode` under high pressure);
* `safe_mode_triggers` — fault classes that trigger a switch to
  `SafeMode`;
* `drift_triggers` — drift envelope thresholds that trigger a switch.

**Why multi-mode is in the schema even before F-D6 is unblocked:** the
schema is forward-compatible. v1 builds may emit a single mode
(`SteadyStateGeneration` for the default profile, `InteractiveTyping`
for the bringup profile). F-D6 adds the runtime-side switching producer.

**Why modes share artifact semantics:** a mode is a *schedule*
optimization, not a *semantics* optimization. Switching modes mid-token
must produce the same final token (modulo determinism class) as not
switching. The `checkpoint_schema_hash` is the same across modes; the
continuation ABI (`InferenceState` shape, slice-id allocation) is the
same; only the slice composition and tile sizes differ.

Amends planv0: `planv0.md` line 1840 defines `SchedulePack` and lines
1881–1883 say "all modes share the same artifact semantics, checkpoint
schema, and continuation ABI." This RFC pins the equality at the
hash level: every mode's `gb_sched_ir.checkpoint_schema_hash` is equal
to `SchedulePack.checkpoint_schema_hash`; every mode's
`continuation_abi_hash` is equal across modes.

### 2.7 InterruptPolicy as part of slice contract

Every slice carries an `InterruptPolicy` ∈ `{ Enabled,
ShortCriticalSection, Disabled }`. This is a per-slice contract because
the cooperative kernel mixes both: the bulk of slices execute with
interrupts enabled (UI must remain responsive); short critical sections
disable interrupts around hardware writes; rare slices (e.g. PersistGroupCommit
write of the commit word) disable interrupts for the smallest possible
window.

The policy informs:

* `max_interrupt_latency` bounds (§2.3);
* `ResourceStateValidation`'s ISR-visible-residency check (§9.1.3);
* lease compatibility — `InterruptMask` leases require `Disabled` or
  `ShortCriticalSection` slices.

**Why per-slice and not per-build:** an `Enabled`-everywhere build
cannot host short critical sections; a `Disabled`-everywhere build is
a UI-killing freeze. Per-slice typing makes the cooperative kernel
correct by construction.

Amends planv0: `planv0.md` line 1826 defines `InterruptPolicy` with
three variants. This RFC pins the v1 set as exactly those three; future
variants require RFC amendment.

### 2.8 YieldKind taxonomy

The closed v1 `YieldKind` set is:

```rust
pub enum YieldKind {
    Micro,
    Frame,
    NeedInput,
    TokenReady,
    Finished,
    Fault,
}
```

* **`Micro`** — a fine-grained yield within a tile loop. The runtime
  may resume immediately or service a pending interrupt. Continuation
  state is in WRAM only.
* **`Frame`** — a yield at a frame boundary. Continuation state is in
  WRAM; UI may run a full frame service before resuming.
* **`NeedInput`** — the inference is waiting on user input (typed
  prompt). Continuation state is in WRAM and may also be persisted to
  SRAM if the build's persistence policy demands it.
* **`TokenReady`** — a token has been emitted. Continuation state is
  in WRAM; persistent sequence state may be committed at this boundary.
* **`Finished`** — generation is complete. The continuation record is
  finalized.
* **`Fault`** — the slice has detected a recoverable or unrecoverable
  fault. Continuation state is consistent enough to recover (or to
  surface a panic).

`YieldCheckClass` describes how the runtime polls the yield-requested
flag within a slice (e.g. once-at-end vs every-N-tiles). The two are
distinct: `YieldKind` says *why* the slice yields; `YieldCheckClass`
says *how* it checks for yield requests within a slice.

Amends planv0: `planv0.md` line 1817 defines `YieldKind` with six
variants. This RFC pins the v1 set as exactly those six; future
variants require RFC amendment.

### 2.9 ExitKind taxonomy

`ExitKind` is the per-slice transition class at slice exit:

```rust
pub enum ExitKind {
    SaveContinuationAndYield,
    TailCall,
    EnterIsr,
    Halt,
    Fault,
}
```

* **`SaveContinuationAndYield`** — the default. The slice writes the
  continuation record and returns control to the cooperative scheduler.
* **`TailCall`** — the slice transfers directly to the next slice
  without yielding (used for tight loops where yielding would be too
  expensive). The next slice's `entry_residency` must match this slice's
  exit residency; no lease may need re-acquisition.
* **`EnterIsr`** — used only by the ISR-entry slice (one per build,
  in Bank0/HRAM). Disallowed for normal slices.
* **`Halt`** — used only by the halt slice (one per build). Used for
  `Finished` and unrecoverable `Fault` paths.
* **`Fault`** — the slice transfers to the fault-handler slice. The
  fault path's residency is part of the slice's `entry_residency`
  declaration.

Amends planv0: `planv0.md` line 1814 lists `ExitKind` as a slice field
without enumerating variants. This RFC pins the v1 enum.

### 2.10 Drift envelope as observable runtime contract

The `DriftEnvelope` is the runtime-observable contract bound to compiled
choices. It carries four metrics:

```rust
pub struct DriftEnvelope {
    pub slice_cycles_p95: Option<u32>,
    pub ui_commit_cycles_p95: Option<u32>,
    pub trace_drop_rate_pct: Option<u8>,
    pub persist_overrun_rate_pct: Option<u8>,
}
```

The compiler emits the **expected** envelope; the runtime emits the
**observed** envelope. `RuntimeDriftMonitor.consecutive_violations`
counts the number of consecutive observation windows in which any
metric exceeds its trigger threshold.

`DriftTrigger` binds a metric to a threshold and an action:

```rust
pub struct DriftTrigger {
    pub metric: DriftMetric,
    pub threshold: u32,
    pub action: DriftAction,
}

pub enum DriftMetric {
    SliceCyclesP95,
    UiCommitCyclesP95,
    TraceDropRatePct,
    PersistOverrunRatePct,
}

pub enum DriftAction {
    ShrinkSlices,
    DropTrace,
    DemoteMode(RuntimeMode),
}
```

**Why drift is part of the schedule:** a cooperative kernel can be
locally safe and globally broken (§12). Drift triggers are the
runtime's response to drift between expected and observed; they are
part of the schedule's contract because they bound the runtime's
behavior. F-D1 implements the runtime side; F-B13 owns the schema.

Amends planv0: `planv0.md` lines 1855–1879 define `RuntimeDriftMonitor`,
`DriftEnvelope`, `DriftTrigger`, `DriftAction`. This RFC pins the v1
fields, and adds the explicit `DriftMetric` enum (planv0 leaves it
implicit).

### 2.11 Stage 10.5 as a separate typed pass producing a separate certificate

Stage 10.5's proof obligations could in principle be inline assertions
inside Stage 10. They are not. Reasons:

1. **Independent re-checkability.** The certificate is consumable by
   `gbf-verify` (an independent, slow-path validator). If the proof
   were inline, there would be no way to externally re-check it
   without re-running Stage 10.
2. **Separation of construction from proof.** Stage 10's job is to
   *produce* a candidate schedule; lease-balance and yield-safety
   obligations are properties of the produced schedule. Folding them
   into Stage 10 would mean every Stage 10 path that emitted a
   schedule would also have to assert these properties locally — a
   maintainability tax with no benefit.
3. **Different cache discipline.** Stage 10's StageCache key includes
   all upstream plan hashes. Stage 10.5's StageCache key includes only
   the Stage 10 product hash. Splitting them lets a Stage 10 cache hit
   skip Stage 10.5 only if Stage 10.5 also hits.
4. **Separate certificate.** The certificate is a typed JSON document
   (`certs/resource_state.cert.json`) under `certs/`, alongside other
   per-stage certificates. It is independent of `sched_ir.json`
   (the Stage 10 product report) and `slice_report.json` (the Stage 10
   per-slice histogram report).

Amends planv0: `planv0.md` line 1885 says "ResourceStateValidation runs
after GbSchedIR and proves...". This RFC pins the proof as a separately
gated typed pass with its own certificate.

### 2.12 Cycle budgets are bounds, not predictions

`hard_cycles_to_safe_point` and `max_interrupt_latency` are **upper
bounds** the schedule guarantees. `soft_target_cycles` is a
**prediction**. F-B14 computes actual cycles against calibration; if
F-B14's actual count exceeds `hard_cycles_to_safe_point`, the schedule
is rejected.

**Why this split:** the schedule's contract with the runtime is that
slices will not exceed their hard bound. The schedule's contract with
F-B14 is that the soft target is an honest prediction (used for
objective satisfaction estimation). Conflating the two would force F-B14
to reject schedules whose typical cycle count is below the bound but
whose worst-case is at the bound — a false-positive failure mode.

### 2.13 No scheduling cost in this stage

F-B13 declares slice budgets in `CycleBudget` units; F-B14 produces
actual cycle counts against the calibration bundle. F-B13 does *not*
consume calibration. This is a deliberate seam: F-B13 must be
calibration-free so that the schedule shape is reproducible across
calibration revisions, and so that F-B14 can re-run cost analysis
against a refreshed calibration without re-running scheduling.

```text
F-NoCalibrationInF-B13:
  SchedulePack contains no observed cycle counts, no calibration hash
  reference (other than the pass-through evidence chain), and no
  calibration-derived field.
```

### 2.14 Repair policy is named-only

```text
F-NoRepairInChunk:
  No diagnostic in sched_ir.json, slice_report.json, or
  certs/resource_state.cert.json carries a RepairProposal source or
  any AuthorizedRelaxation operation.
  PolicySource ⊆ {TargetDefault, ProfileDefault, CompileRequestOverride,
                  HintBundle, Calibration} (per F-B2 §2.7).

F-RepairKnobsWired:
  CompileKnobs schema (resolved by F-B2 Stage 0.5) carries any
  schedule-relevant knobs as named-only hooks. F-B16 unblocks
  RepairProposal source. No knob is *consumed* by F-B13 in v1
  beyond reading the resolved value.
```

### 2.15 No "quick fix" defaults

If `GbSchedIR` would only succeed by silently filling in a default slice
boundary, lease shape, residency epoch, or mode entry, it fails. If
`ResourceStateValidation` would only succeed by silently waiving a
proof obligation, it fails. Every product field derives from a
hash-bound input or fails loudly.

### 2.16 Single-mode v1 closure surface

For the chunk-closure fixture, `SchedulePack` v1 emits **one** mode per
build (`SchedulePack.modes.len() == 1`, mode key
`RuntimeMode::SteadyStateGeneration` for the Default profile or
`RuntimeMode::InteractiveTyping` for the Bringup profile). The schema
accommodates multiple modes; the closure gate exercises the multi-mode
schema only in fixture builds.

Amends planv0: `planv0.md` line 1881 leaves the mode cardinality plural
without specifying v1 closure. This RFC pins v1 closure to "modes ≥ 1,
multi-mode exercised only in fixtures" while keeping the schema plural.

### 2.17 Drift envelope v1 minimum

For v1 closure, `RuntimeDriftMonitor.expected.slice_cycles_p95` is
required to be `Some` (the schedule must declare an expected p95 cycle
budget per slice). The other three fields may be `None` until the
runtime measurement bus is wired in F-D1.

```text
F-DriftV1Minimum:
  RuntimeDriftMonitor.expected.slice_cycles_p95.is_some()
  ∧ RuntimeDriftMonitor.observed = DriftEnvelope::all_none()  // compile-time
```

Amends planv0: `planv0.md` line 1861 leaves `slice_cycles_p95` optional.
This RFC pins it as required for v1 closure.

### 2.18 Determinism and cache discipline

Both stage products are fully content-addressed:

```text
sched_ir_self_hash := DomainHash(
    "gbf-codegen", "GbSchedIR", "v1",
    CanonicalJson(SchedulePack after canonical sort))

slice_report_self_hash := DomainHash(
    "gbf-codegen", "SliceReport", "v1",
    CanonicalJson(SliceReportBody after canonical sort))

resource_state_cert_self_hash := DomainHash(
    "gbf-codegen", "ResourceStateCertificate", "v1",
    CanonicalJson(ResourceStateCertBody after canonical sort))
```

Determinism axioms:

```text
F-Det-SchedIr:
  Same GbInferIR + ObservationPlan + RangePlan + StoragePlan
       + SramPagePlan + RomWindowPlan + OverlayPlan + ArenaPlan
       + ResolvedCompilePolicy + RuntimeChromeBudget
  ⇒ byte-identical SchedulePack and sched_ir.json.

F-Det-ResourceState:
  Same SchedulePack
  ⇒ byte-identical certs/resource_state.cert.json.
```

`StageCache` keys (K10, K10.5) participate in the determinism witness:
two builds with identical input hashes hit the cache; one byte changed
in any input misses.

### 2.19 Schema versioning

```text
sched_ir.v1
slice_report.v1
resource_state.cert.v1
```

Schema bumps follow F-B2/F-B4 §10's compatibility rules. A v1 → v2 bump
is not allowed unless a later RFC explicitly amends this RFC.

### 2.20 ISR-visible semantics: annotation-driven now, computed later

Stage 10.5's "no ISR-visible path depends on leased switchable state"
check is annotation-driven in v1: it trusts the slice's declared
`interrupt_policy` and `entry_residency`, plus the upstream
`KernelResidency` from F-B10, plus the upstream `ArenaSlot.named` from
F-B12. It does *not* compute transitive reachability through far-call
edges, because far-call legalization has not happened yet (F-B15).

The handshake with F-B15 is:

* F-B13 Stage 10.5 catches violations that are visible at the schedule
  level (a slice with `interrupt_policy = Enabled` whose
  `entry_residency != Bank0` or `Common(_)` is a hard reject).
* F-B15 Reachability catches violations that emerge after far-call
  legalization (e.g. an ISR vector that transitively reaches a thunk
  whose target is in an Expert bank).

Builds that pass Stage 10.5 but fail F-B15 Reachability are signaled to
F-B16 (BLOCKED) for repair-loop iteration.

Amends planv0: `planv0.md` line 1889 says
"no ISR-visible path depends on leased switchable state" without
specifying compile-time vs runtime locality. This RFC pins it as a
two-stage handshake: annotation now, computed later.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the F-B2/F-B4,
F-B3/F-B5, and F-B11/F-B12 glossary inheritance.

| Term                       | Definition                                                                                  |
|----------------------------|---------------------------------------------------------------------------------------------|
| Slice                      | The cooperative-kernel scheduling unit. A sequence of `SchedOp`s with bounded interrupt latency, declared interrupt policy, declared entry residency, and a yielding or fault-class exit. |
| SliceId                    | Stable identifier for one `SchedSlice`. Content-addressed across runs. |
| SchedOp                    | One operation inside a slice. Includes load, store, tile-loop, acquire-lease, release-lease, yield, and exit operations. |
| ResourceLease              | A typed acquire/release pair on `RomWindow`, `SramPage`, `Overlay`, or `InterruptMask`. Mirrors F-A4's `BankLease` ABI at compile time. |
| LeaseId                    | Stable identifier for one `ResourceLease`. |
| ResourceLeaseKind          | Closed enum of lease kinds: `RomWindow(RomWindowBinding)`, `SramPage(SramPageBinding)`, `Overlay(OverlayId)`, `InterruptMask(InterruptPolicy)`. |
| ResourceVector             | Per-slice tally of bank_switches, sram_page_switches, trace_bytes, persist_bytes, overlay_installs. |
| ResidencyEpoch             | Contiguous run of slices sharing `RomWindowBinding`, `OverlayId`, and `Residency`. Crossing an epoch boundary is a typed event. |
| EpochId                    | Stable identifier for one `ResidencyEpoch`. |
| SchedulePack               | The multi-mode schedule artifact. Map keyed by `RuntimeMode` whose values are per-mode `GbSchedIR` instances and per-mode `Vec<ResidencyEpoch>`. Carries one shared `checkpoint_schema_hash` and one `ModeSwitchPolicy`. |
| RuntimeMode                | Closed enum: `InteractiveTyping`, `SteadyStateGeneration`, `TraceHeavyDebugging`, `SafeMode`. |
| ModeSwitchPolicy           | Bounds for legal mode switches: `legal_switch_points`, `legal_epoch_boundaries`, `ui_pressure_thresholds`, `safe_mode_triggers`, `drift_triggers`. |
| RuntimeDriftMonitor        | Compiler-side `expected` envelope plus runtime-side `observed` envelope plus `consecutive_violations` count. |
| DriftEnvelope              | Four-field summary: `slice_cycles_p95`, `ui_commit_cycles_p95`, `trace_drop_rate_pct`, `persist_overrun_rate_pct`. |
| DriftTrigger               | `(metric, threshold, action)` triple binding a `DriftMetric` to a `DriftAction`. |
| DriftAction                | Closed enum: `ShrinkSlices`, `DropTrace`, `DemoteMode(RuntimeMode)`. |
| DriftMetric                | Closed enum mirroring `DriftEnvelope` field set. |
| YieldKind                  | Closed enum: `Micro`, `Frame`, `NeedInput`, `TokenReady`, `Finished`, `Fault`. |
| YieldCheckClass            | How the runtime polls the yield-requested flag inside a slice. |
| ExitKind                   | Closed enum: `SaveContinuationAndYield`, `TailCall`, `EnterIsr`, `Halt`, `Fault`. |
| InterruptPolicy            | Closed enum: `Enabled`, `ShortCriticalSection`, `Disabled`. |
| CycleBudget                | A typed cycle count. Used for `hard_cycles_to_safe_point`, `soft_target_cycles`, `max_interrupt_latency`. |
| ResourceStateCertificate   | The Stage 10.5 product. Machine-checkable JSON certificate that records every proof obligation discharged in §9.1. |
| Lease-flow analysis        | Typed analysis over `SchedSlice` ops that establishes lease balance and yield-safety. Not a runtime simulation; it is symbolic. |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4, §4 of F-B3/F-B5, and §4 of F-B11/F-B12
(Hash256, Outcome, Severity, Stage, ReportSchema, Result, Option,
NonEmptyList, SortedBy, DomainHash, SelfHash, CanonicalJson, ZERO_HASH,
null policy, ValidationOrigin extensions). Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3 | Stage4 | Stage5
  | Stage6 | Stage7 | Stage8 | Stage8_5 | Stage9
  | Stage10        -- new (GbSchedIR)
  | Stage10_5      -- new (ResourceStateValidation)

ReportSchema :=
  ... (inherited)
  | "sched_ir.v1"               -- new
  | "slice_report.v1"           -- new
  | "resource_state.cert.v1"    -- new

ValidationOrigin :=
  ... (inherited)
  | SchedIrConstruction         -- new
  | ResourceStateValidation     -- new
  | ScheduleModePack            -- new
  | RuntimeDriftMonitor         -- new

ValidationCode :=
  ... (inherited)
  | SCHED-* codes (§15.1)
  | LEASE-* codes (§15.2)
  | RES-* codes (§15.3)
  | MODE-* codes (§15.4)
  | DRIFT-* codes (§15.5)

CycleBudget := u32  -- cycles, no overflow at the upper bound for any
                       practical slice (a 2^32 cycle slice would take
                       ~1024 seconds at 4 MHz; we cap practical slices
                       at u16 cycles, but the field is u32 to allow
                       drift-envelope thresholds to live in the same
                       type).

ResourceVector := struct {
  bank_switches:      u16,
  sram_page_switches: u16,
  trace_bytes:        u16,
  persist_bytes:      u16,
  overlay_installs:   u8,
}
```

`SortedBy` is used pervasively in this RFC for canonical ordering of
arrays whose order is not semantically meaningful (slice id sets, lease
id sets, epoch id sets). The sort key is documented per use site.

## 5. Authority rules

This RFC is the authority for the following:

* `SchedSlice`, `SchedOp`, `ResourceLease`, `ResourceLeaseKind`,
  `ResourceVector`, `ResidencyEpoch`, `SchedulePack`, `ModeSwitchPolicy`,
  `RuntimeMode`, `RuntimeDriftMonitor`, `DriftEnvelope`, `DriftTrigger`,
  `DriftAction`, `DriftMetric`, `YieldKind`, `YieldCheckClass`,
  `ExitKind`, `InterruptPolicy`, `LeaseId`, `EpochId`, `SliceId`,
  `CycleBudget`, `ResourceStateCertificate` shapes.
* `sched_ir.v1`, `slice_report.v1`, `resource_state.cert.v1` schemas.
* The lease-balance, yield-safety, ISR-visible-residency, and
  overlay/bank-shadow-consistency proof obligations and their certificate
  evidence shapes.
* The mode-pack equality invariants (`checkpoint_schema_hash`,
  continuation ABI hash) and the legal-switch-point semantics.
* The drift-envelope handshake schema (compiler-side `expected`,
  runtime-side `observed`, `consecutive_violations`).
* `K10` and `K10.5` StageCache key construction.

This RFC inherits and does not modify:

* `ReportEnvelope<R>` shape (F-B2/F-B4 §4).
* Hashing primitives, canonical JSON, null policy (F-B2/F-B4 §1, §2.5).
* `ValidationDiagnostic` shape (F-B2/F-B4 §5).
* `QuantGraph`, `GbInferIR` shapes (F-B3/F-B5).
* `OverlayId`, `OverlayInstall`, `ArenaSlot`, `NamedArena` shapes
  (F-B11/F-B12).
* `Materialization`, `LifetimeClass`, `AliasClassId`, `StorageClass`
  shapes (F-B8).
* `SramPageBinding`, `RomWindowBinding`, `KernelResidency`, `Residency`
  shapes (F-B9, F-B10).
* `SemanticCheckpointId`, `TraceProbeId` shapes (F-B6, F-A3).
* `BankLease` / `BankGuard` runtime ABI (F-A4).

## 6. Pipeline state machine

The chunk-level state machine for a build under F-B13 is:

```text
state init
  -> Stage 10 GbSchedIR start
       precondition: every upstream plan has emitted a content-
       addressed product whose hash is recorded in the stage inputs.

  Stage 10 path:
    -> residency-epoch construction
    -> slice formation
    -> lease binding (acquire/release pairing)
    -> checkpoint pinning (semantic_checkpoint_schema_hash inheritance)
    -> mode-pack assembly
    -> drift-envelope binding
    -> emit SchedulePack, ReportEnvelope<SchedIrReportBody>,
            ReportEnvelope<SliceReportBody>
    -> StageCache: write K10 entry on success
    -> next state: Stage 10.5 (always run; even cached Stage 10 hits
                                must be re-validated unless K10.5 also
                                hits)

  -> Stage 10.5 ResourceStateValidation start
       precondition: Stage 10 has emitted a SchedulePack hash.

  Stage 10.5 path:
    -> typed lease-flow analysis (proves lease balance)
    -> typed yield-safety analysis (proves no illegal yield crossings)
    -> typed ISR-visible-residency analysis (annotation-driven; pairs
                                              with F-B15 Reachability
                                              for computed proof)
    -> typed overlay/bank-shadow consistency analysis
    -> emit ResourceStateCertificate,
            ReportEnvelope<ResourceStateCertBody>
    -> StageCache: write K10.5 entry on success
    -> next state: Stage 11 (F-B14)

  on failure at any sub-pass:
    -> emit failure report with hard diagnostics
    -> StageCache may write failure memo (per F-B2/F-B4 §2.6)
    -> halt build
```

Stage 10.5 *must* run on every build. A Stage 10 cache hit does *not*
imply Stage 10.5 may be skipped; the certificate is independently
content-addressed against the Stage 10 product hash. If both K10 and
K10.5 hit, both products are replayed verbatim.

## 7. Report envelope

This RFC inherits the `ReportEnvelope<R>` shape and self-hash convention
verbatim from F-B2/F-B4. Every report and every certificate is wrapped
in an envelope:

```rust
pub struct ReportEnvelope<R> {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}
```

Public JSON remains flat: `schema`, `schema_version`, `outcome`, and
`report_self_hash` are top-level fields, followed by the body fields.
All bodies use `#[serde(deny_unknown_fields)]`. All enums use an
explicitly tagged representation. F-B13 reports reject any diagnostic
whose severity is `Soft` (per F-B2/F-B4 §2.13).

The three report bodies introduced by this chunk are:

* `SchedIrReportBody` — body of `sched_ir.json`. Section §13.1.
* `SliceReportBody` — body of `slice_report.json`. Section §13.2.
* `ResourceStateCertBody` — body of `certs/resource_state.cert.json`.
  Section §13.3.

The certificate is a report. It uses the same envelope rules. The only
distinguishing convention is its location: certificates live under
`certs/` while ordinary reports live at the build artifacts root.

## 8. Stage 10 contract: GbSchedIR

### 8.1 Type-level contract

This subsection lists the public types Stage 10 introduces. Every type
is content-addressed; every field is typed; every closed enum is
exhaustively enumerated.

#### 8.1.1 SliceId, LeaseId, EpochId, CycleBudget

```rust
/// Stable identifier for a slice. Content-addressed across runs.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd,
         serde::Serialize, serde::Deserialize)]
pub struct SliceId(pub u32);

/// Stable identifier for a resource lease.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd,
         serde::Serialize, serde::Deserialize)]
pub struct LeaseId(pub u32);

/// Stable identifier for a residency epoch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd,
         serde::Serialize, serde::Deserialize)]
pub struct EpochId(pub u32);

/// A typed cycle budget. u32 to allow drift envelope thresholds in the
/// same type; per-slice slice budgets are checked against u16::MAX
/// during Stage 10 self-consistency.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd,
         serde::Serialize, serde::Deserialize)]
pub struct CycleBudget(pub u32);
```

`SliceId`, `LeaseId`, `EpochId` are assigned in canonical order during
construction (§8.3); the assignment is deterministic and stable.

#### 8.1.2 SchedSlice

```rust
pub struct SchedSlice {
    pub id: SliceId,
    pub ops: Vec<SchedOp>,
    pub hard_cycles_to_safe_point: CycleBudget,
    pub soft_target_cycles: CycleBudget,
    pub max_interrupt_latency: CycleBudget,
    pub resources: ResourceVector,
    pub live_wram: Vec<ArenaSlotRef>,
    pub live_sram: Vec<ArenaSlotRef>,
    pub yield_kind: YieldKind,
    pub yield_check: YieldCheckClass,
    pub entry_residency: Residency,
    pub interrupt_policy: InterruptPolicy,
    pub required_leases: Vec<LeaseId>,
    pub exit_kind: ExitKind,
    pub semantic_checkpoint_pins: Vec<SemanticCheckpointId>,
    pub trace_probe_pins: Vec<TraceProbeId>,
    pub successors: Vec<SliceId>,
}

pub struct ArenaSlotRef {
    /// Reference to a slot in F-B12's ArenaPlan by id.
    pub arena: ArenaId,
    pub slot_index: u32,
}
```

**Field semantics:**

* `id` — see §8.1.1.
* `ops` — the operation sequence. Last op must be a yield-class or
  exit-class op (§8.2).
* `hard_cycles_to_safe_point` — upper bound on cycles from slice entry
  to next safe point (yield-or-exit). Required to be ≥
  `max_interrupt_latency`. F-B14 verifies the bound at the calibration
  level; F-B13 checks the inequality.
* `soft_target_cycles` — predicted typical cycles. Required to be ≤
  `hard_cycles_to_safe_point`.
* `max_interrupt_latency` — upper bound on cycles between an interrupt
  request and ISR entry while this slice is executing. For
  `interrupt_policy = Enabled` slices, this is bounded above by hardware;
  for `ShortCriticalSection`, this is the bound on the critical section
  length; for `Disabled`, this is the bound on cycles until interrupts
  are re-enabled.
* `resources` — per-slice resource vector. See §8.1.5.
* `live_wram` / `live_sram` — `ArenaSlotRef`s whose `LifetimeClass`
  covers the slice's lifetime. Used by F-B15 to lay down section
  references.
* `yield_kind`, `yield_check`, `entry_residency`, `interrupt_policy`,
  `exit_kind` — see §8.1.3 / §8.1.4.
* `required_leases` — leases the slice depends on at entry. Subset of
  the leases acquired in the dominating slice path (§8.4.5).
* `semantic_checkpoint_pins` — `SemanticCheckpointId`s pinned at slice
  boundaries. Inherited from F-B6's `ObservationPlan`. The slice may
  pin a checkpoint at entry, exit, or both, depending on the
  observation mode.
* `trace_probe_pins` — `TraceProbeId`s pinned within the slice's ops.
  Inherited from F-B6.
* `successors` — successor slice ids. The slice graph is a DAG; cycles
  (across yields) are represented at the residency-epoch level, not
  the slice level. For `ExitKind::SaveContinuationAndYield`, successors
  is the set of resume-target slice ids; for `TailCall`, it is the
  single tail-call target; for `Halt`, it is empty.

#### 8.1.3 YieldKind, YieldCheckClass, ExitKind, InterruptPolicy

```rust
pub enum YieldKind {
    Micro,
    Frame,
    NeedInput,
    TokenReady,
    Finished,
    Fault,
}

pub enum YieldCheckClass {
    /// Polled exactly once before the slice's terminating yield op.
    OnceAtEnd,
    /// Polled every N tile iterations within the slice.
    EveryNTiles { n: u8 },
    /// Polled at every load-store pair boundary.
    EveryLoadStore,
    /// Slice does not poll; ExitKind ∈ {TailCall, Halt}.
    NoPoll,
}

pub enum ExitKind {
    SaveContinuationAndYield,
    TailCall,
    EnterIsr,
    Halt,
    Fault,
}

pub enum InterruptPolicy {
    Enabled,
    ShortCriticalSection,
    Disabled,
}
```

#### 8.1.4 Residency

```rust
/// Inherited from F-B10 RomWindowPlan.
pub enum Residency {
    Bank0,
    Common(BankId),
    Expert(ExpertId, BankId),
}
```

`Residency` appears in `SchedSlice.entry_residency` and in
`ResidencyEpoch.residency`. They must agree (§8.4.3).

#### 8.1.5 ResourceVector

```rust
pub struct ResourceVector {
    pub bank_switches: u16,
    pub sram_page_switches: u16,
    pub trace_bytes: u16,
    pub persist_bytes: u16,
    pub overlay_installs: u8,
}
```

Per-slice tally, used by F-B14 for cost analysis. The values are
**static** counts derived from the slice's op sequence, not measured
runtime counts.

#### 8.1.6 ResourceLease, ResourceLeaseKind

```rust
pub struct ResourceLease {
    pub id: LeaseId,
    pub kind: ResourceLeaseKind,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
}

pub enum ResourceLeaseKind {
    RomWindow(RomWindowBinding),
    SramPage(SramPageBinding),
    Overlay(OverlayId),
    InterruptMask(InterruptPolicy),
}
```

`yield_safe` is determined by the kind × yield-kind matrix (§8.4.4):

| `kind`                          | yields legal?                         |
|---------------------------------|---------------------------------------|
| `RomWindow(_)`                  | no                                    |
| `SramPage(_)`                   | no                                    |
| `Overlay(_)`                    | yes if eviction policy permits, else no |
| `InterruptMask(Disabled)`       | no                                    |
| `InterruptMask(ShortCriticalSection)` | no                              |
| `InterruptMask(Enabled)`        | trivially yes (no lease semantics)    |

`InterruptMask(Enabled)` is included for symmetry but is a no-op; it is
the default and does not actually alter machine state. Slices with
`interrupt_policy = Enabled` do not acquire an `InterruptMask(Enabled)`
lease.

#### 8.1.7 ResidencyEpoch

```rust
pub struct ResidencyEpoch {
    pub id: EpochId,
    pub rom_window: RomWindowBinding,
    pub overlay: Option<OverlayId>,
    pub residency: Residency,
    pub slices: Vec<SliceId>,
}
```

Every slice belongs to exactly one epoch; epochs do not overlap; the
union of all epochs' slice sets equals the schedule's slice set
(§8.4.6).

#### 8.1.8 SchedOp

```rust
pub enum SchedOp {
    /// Acquire a resource lease at this point in the slice.
    AcquireLease { lease: LeaseId },
    /// Release a resource lease at this point in the slice.
    ReleaseLease { lease: LeaseId },
    /// Perform an overlay install inside the slice.
    OverlayInstall { install: InstallId },
    /// Bank-switch the ROM window. Only legal when holding a RomWindow
    /// lease whose binding matches the new bank.
    BankSwitch { from: BankId, to: BankId },
    /// SRAM-page-switch.
    SramPageSwitch { from: u8, to: u8 },
    /// Compute kernel call (refers to F-H1 KernelSpecId; opaque here).
    KernelCall { spec: KernelSpecId, tile_index: TileIndex },
    /// Load from arena to scratch (typed by ValueId).
    Load { value: ValueId, src: ArenaSlotRef, dst: ScratchSlot },
    /// Store from scratch to arena (typed by ValueId).
    Store { value: ValueId, src: ScratchSlot, dst: ArenaSlotRef },
    /// Effect-edge transition (sequence-state mutation, RNG progression).
    /// The EffectId is from F-B5 GbInferIR.
    Effect { effect: EffectId },
    /// Trace probe event. TraceProbeId from F-B6 ObservationPlan.
    TraceProbe { probe: TraceProbeId },
    /// Semantic checkpoint pin. SemanticCheckpointId from F-B6.
    SemanticCheckpoint { checkpoint: SemanticCheckpointId },
    /// Persistent-state commit. CommitGroupId from F-B8 StoragePlan.
    PersistCommit { group: CommitGroupId },
    /// Yield. Last op of yielding slices.
    Yield { kind: YieldKind },
    /// Tail call. Last op of TailCall-exit slices.
    TailCall { target: SliceId },
    /// Enter ISR. Last op of EnterIsr-exit slices (ISR entry trampoline).
    EnterIsr { vector: InterruptVector },
    /// Halt. Last op of Halt-exit slices.
    Halt,
    /// Fault. Last op of Fault-exit slices.
    Fault { code: FaultCode },
}
```

**Op shape constraints:**

* `AcquireLease { lease }` is paired with exactly one
  `ReleaseLease { lease }` on every reachable slice path (§9.1.1).
* `OverlayInstall { install }` requires the `OverlayLeaseShape` of the
  install to be satisfied by the surrounding lease scope. The install
  emits the bytes; F-B15 lowers the install to the runtime overlay
  loader.
* `BankSwitch { from, to }` requires a `RomWindow` lease for `to` to
  have been acquired *before* the switch and a `RomWindow` lease for
  `from` to have been released *before* the switch. The two events
  may be expressed as a single `AcquireLease`/`ReleaseLease` pair
  (release of `from`, acquire of `to`) at the lease level.
* `KernelCall` references an opaque `KernelSpecId` (Epic H). The slice
  declares which kernel; F-B15 selects the implementation.
* `Load` / `Store` reference `ArenaSlotRef`s; the slot must appear in
  the slice's `live_wram` or `live_sram`.
* `Yield`, `TailCall`, `EnterIsr`, `Halt`, `Fault` are terminal ops;
  exactly one of them ends every slice.
* `Effect`, `TraceProbe`, `SemanticCheckpoint`, `PersistCommit` may
  appear anywhere within the slice (not as terminal ops).

#### 8.1.9 ScratchSlot

```rust
/// A typed reference to a register or short-lived WRAM scratch byte.
/// F-B13 represents scratch as a typed handle; F-B15 selects the
/// concrete register or address.
pub struct ScratchSlot {
    pub kind: ScratchKind,
    pub width: ScratchWidth,
    pub class: ScratchClass,
}

pub enum ScratchKind { RegA, RegB, RegC, RegD, RegE, RegH, RegL,
                       RegBC, RegDE, RegHL, WramByte, WramWord }

pub enum ScratchWidth { U8, U16, I16, I32 }

pub enum ScratchClass { CalleeSavedAcrossKernelCall,
                        CallerSavedAcrossKernelCall,
                        ContiguousAcrossYield, /* others */ }
```

`ScratchSlot` is intentionally typed but register-class-free at this
stage; F-B15's register allocator selects concrete registers within
the slice's contract.

#### 8.1.10 SchedulePack

```rust
pub struct SchedulePack {
    pub modes: BTreeMap<RuntimeMode, GbSchedIR>,
    pub epochs: BTreeMap<RuntimeMode, Vec<ResidencyEpoch>>,
    pub leases: BTreeMap<RuntimeMode, Vec<ResourceLease>>,
    pub checkpoint_schema_hash: Hash256,
    pub continuation_abi_hash: Hash256,
    pub switch_policy: ModeSwitchPolicy,
    pub drift_monitor: RuntimeDriftMonitor,
}

pub struct GbSchedIR {
    pub slices: Vec<SchedSlice>,
    pub entry_slice: SliceId,
    pub mode: RuntimeMode,
}

pub enum RuntimeMode {
    InteractiveTyping,
    SteadyStateGeneration,
    TraceHeavyDebugging,
    SafeMode,
}
```

**Pack-level invariants** (§8.4.7):

* `modes.keys()` is non-empty.
* `modes.keys() == epochs.keys() == leases.keys()` (every mode has its
  own per-mode `GbSchedIR`, `Vec<ResidencyEpoch>`, and lease set).
* every mode's `GbSchedIR.checkpoint_schema_hash`-derived hash equals
  `SchedulePack.checkpoint_schema_hash` (the schema is shared across
  modes).
* every mode's continuation ABI shape (slice id allocation pattern,
  arena slot membership, harness command/result block addressing) yields
  the same `continuation_abi_hash`.

#### 8.1.11 ModeSwitchPolicy, RuntimeDriftMonitor, DriftEnvelope, DriftTrigger, DriftAction, DriftMetric

```rust
pub struct ModeSwitchPolicy {
    pub legal_switch_points: Vec<SemanticCheckpointId>,
    pub legal_epoch_boundaries: Vec<EpochId>,
    pub ui_pressure_thresholds: Vec<UiPressureThreshold>,
    pub safe_mode_triggers: Vec<SafeModeTrigger>,
    pub drift_triggers: Vec<DriftTrigger>,
}

pub struct UiPressureThreshold {
    pub metric: UiPressureMetric,
    pub threshold: u32,
    pub action: ModeSwitchAction,
}

pub enum UiPressureMetric {
    FrameJitterMs,
    VideoCommitLatencyCycles,
    KeyboardInputLatencyCycles,
}

pub enum ModeSwitchAction {
    SwitchTo(RuntimeMode),
    Stay,
}

pub enum SafeModeTrigger {
    FaultClass(FaultClass),
    LivenessTimeout,
    PersistentRecordCorruption,
    HarnessAbort,
}

pub struct RuntimeDriftMonitor {
    pub expected: DriftEnvelope,
    pub observed: DriftEnvelope,
    pub consecutive_violations: u8,
    pub window_frames: u16,
}

pub struct DriftEnvelope {
    pub slice_cycles_p95: Option<u32>,
    pub ui_commit_cycles_p95: Option<u32>,
    pub trace_drop_rate_pct: Option<u8>,
    pub persist_overrun_rate_pct: Option<u8>,
}

pub struct DriftTrigger {
    pub metric: DriftMetric,
    pub threshold: u32,
    pub action: DriftAction,
}

pub enum DriftMetric {
    SliceCyclesP95,
    UiCommitCyclesP95,
    TraceDropRatePct,
    PersistOverrunRatePct,
}

pub enum DriftAction {
    ShrinkSlices,
    DropTrace,
    DemoteMode(RuntimeMode),
}
```

**Drift contract:**

* `expected` is set by the compiler (Stage 10).
* `observed` is set by the runtime (F-D1) and is `DriftEnvelope::all_none()`
  in the compile-time product.
* `consecutive_violations` is initialized to 0 in the compile-time
  product.
* `window_frames` is the runtime measurement window in frames (typically
  60 = 1 second); it is set by the compiler.

### 8.2 Operation contract

Every `SchedSlice.ops` array follows this grammar:

```text
slice_ops := pre_ops* tile_loop? post_ops* terminal_op
pre_ops := AcquireLease | OverlayInstall | TraceProbe |
           SemanticCheckpoint | Effect
tile_loop := KernelCall | Load | Store
post_ops := ReleaseLease | TraceProbe | SemanticCheckpoint |
            PersistCommit | Effect
terminal_op := Yield | TailCall | EnterIsr | Halt | Fault
```

**Op-level rules:**

1. The first op of a slice must be either an `AcquireLease`,
   `OverlayInstall`, or `KernelCall` (or the `EnterIsr` op for the
   ISR-entry slice). `Effect` and `TraceProbe` may also appear at
   slice entry but are not the first op of any non-trivial slice.
2. `AcquireLease` and `ReleaseLease` of the same `LeaseId` form an
   acquire-release pair (§9.1.1). Pairs may be nested but never
   cross slice boundaries except through the slice graph (§9.1.1.2).
3. `OverlayInstall` may only appear inside a slice whose surrounding
   lease scope satisfies the install's `OverlayLeaseShape`.
4. `KernelCall` requires the slice's `entry_residency` to match the
   kernel's `KernelResidency` (Bank0Fixed → Bank0; WramOverlay →
   Bank0 with overlay loaded; CoResidentSwitchable → Common(_) for the
   bank that holds the kernel and its data).
5. `Load`/`Store` operations operate on `ArenaSlot`s that are members
   of the slice's `live_wram` or `live_sram`.
6. `Yield { kind }` is permitted only when the slice's outstanding lease
   set is consistent with `yield_safe = true` for every member (§9.1.2).
7. `TailCall { target }` requires the target slice's
   `entry_residency` to match this slice's exit residency, and requires
   no lease to need re-acquisition (the leases held at exit are exactly
   the leases the target requires at entry).
8. `Halt`, `Fault`, `EnterIsr` are terminal-only.
9. Op ordering within a slice is canonical: pre-ops appear before
   tile-loop ops; tile-loop ops appear before post-ops; the terminal
   op is last.

### 8.3 Construction order

Stage 10's pure core constructs the `SchedulePack` in this order:

1. **Residency epoch construction.** From `RomWindowPlan` and
   `OverlayPlan`, construct the set of `(RomWindowBinding, Option<OverlayId>,
   Residency)` triples that define epochs. Each unique triple becomes
   one `ResidencyEpoch`. EpochId is assigned in canonical order
   (sort key: `(rom_window.bank_id, overlay.unwrap_or(0),
   residency.discriminant())`).

2. **Slice formation.** From `GbInferIR` ops, `RangePlan` reduction
   structure, `StoragePlan` materialization decisions, `ArenaPlan`
   slot assignments, walk the IR in topological order and emit slices.
   Slice boundaries are inserted at:
   * tile-loop iterations crossing `hard_cycles_to_safe_point` budgets
     (per `RangePlan` tile sizes);
   * residency-class transitions (Bank0 → Common, Common → Expert);
   * `ObservationPlan` semantic-checkpoint pin sites that require
     yield-safe boundaries;
   * mode-switch points (`SchedulePack.switch_policy.legal_switch_points`).
   SliceId is assigned in canonical order (sort key: topological
   index over the IR walk).

3. **Lease binding.** For every slice, derive the set of leases
   acquired and released. A lease is acquired at the first slice that
   needs the resource and released at the last slice (in topological
   order) that uses it before a successor lacks it. LeaseId is
   assigned in canonical order (sort key: `(acquired_in, kind.discriminant(),
   resource_id)`).

4. **Acquire/Release op insertion.** For every lease, insert an
   `AcquireLease { lease }` SchedOp at the start of `acquired_in` and a
   `ReleaseLease { lease }` SchedOp at the end of `released_in`. Nested
   acquires (where one slice acquires multiple leases) order by lease
   id ascending.

5. **Checkpoint pinning.** For every `SemanticCheckpointId` in
   `ObservationPlan`'s pin set, find the slice whose op sequence aligns
   with the checkpoint and pin it via `slice.semantic_checkpoint_pins`
   and a `SemanticCheckpoint { checkpoint }` SchedOp. Similarly for
   `TraceProbeId`s.

6. **Mode-pack assembly.** For each `RuntimeMode` in
   `ResolvedCompilePolicy.requested_runtime_modes`, run steps 2–5 with
   per-mode tile sizes, yield spacing, and trace density. Assemble the
   per-mode `GbSchedIR`, `Vec<ResidencyEpoch>`, and `Vec<ResourceLease>`
   into `SchedulePack`.

7. **ModeSwitchPolicy assembly.** From `ResolvedCompilePolicy.knobs.mode`,
   `ObservationPlan`, and the per-mode epochs, derive
   `legal_switch_points`, `legal_epoch_boundaries`,
   `ui_pressure_thresholds`, `safe_mode_triggers`, `drift_triggers`.

8. **Drift envelope binding.** From `ResolvedCompilePolicy.knobs.drift`
   and the schedule's slice budgets, derive
   `RuntimeDriftMonitor.expected`. `observed` is initialized to
   `DriftEnvelope::all_none()`. `consecutive_violations` is 0.

9. **Self-consistency check.** Run §8.4 self-consistency rules; on
   violation, emit hard diagnostics and fail.

10. **Hashing.** Compute `checkpoint_schema_hash` from the artifact's
    `SemanticCheckpointSchema` (consumed by hash, not redefined).
    Compute `continuation_abi_hash` from the cross-mode invariants of
    `gbf-abi::InferenceState` shape and the slice id allocation pattern.

The construction is deterministic: same inputs ⇒ byte-identical
`SchedulePack`.

### 8.4 Self-consistency rules

These rules are enforced inside Stage 10 (not Stage 10.5; the proof
obligations are different — see §9). Every violation is a hard
diagnostic.

#### 8.4.1 required_leases ⊆ acquired-in-scope

For every slice `s`, `s.required_leases` must be a subset of the leases
acquired by some slice path leading to `s`:

```text
F-RequiredLeasesSubset:
  ∀ s ∈ SchedulePack[mode].slices.
    s.required_leases ⊆ ⋃ { l ∈ leases | l.acquired_in dominates s
                                       ∧ ¬(l.released_in dominates s) }
```

Diagnostic: `LEASE-RequiredLeaseNotAcquired`.

#### 8.4.2 live_wram / live_sram align with ArenaSlot lifetimes

For every `ArenaSlotRef r ∈ s.live_wram ∪ s.live_sram`, the underlying
`ArenaSlot.lifetime_class` must cover the slice `s`:

```text
F-LiveSlotLifetime:
  ∀ s ∈ SchedulePack[mode].slices, ∀ r ∈ s.live_wram ∪ s.live_sram.
    let slot = ArenaPlan.lookup(r.arena, r.slot_index) in
    slot.lifetime_class.covers(s.lifetime_class)
```

Where `LifetimeClass.covers` is the partial order:

```text
Persistent ⊐ Session ⊐ Token ⊐ ResumeWindow ⊐ Slice
```

A `Slice`-class slot may be live in only one slice; a `ResumeWindow`-class
slot may be live across slices that share a yield boundary; etc.

Diagnostic: `SCHED-LiveSlotLifetimeMismatch`.

#### 8.4.3 entry_residency consistent with ResidencyEpoch

For every slice `s`, `s.entry_residency` must equal the residency of
the epoch that contains `s`:

```text
F-EntryResidencyEpoch:
  ∀ s ∈ SchedulePack[mode].slices.
    let e = SchedulePack[mode].epochs.find(|e| s.id ∈ e.slices) in
    s.entry_residency == e.residency
```

Diagnostic: `RES-EntryResidencyMismatch`.

#### 8.4.4 hard_cycles_to_safe_point ≥ max_interrupt_latency

```text
F-HardLeMaxLatency:
  ∀ s ∈ SchedulePack[mode].slices.
    s.hard_cycles_to_safe_point >= s.max_interrupt_latency

F-SoftLeHard:
  ∀ s ∈ SchedulePack[mode].slices.
    s.soft_target_cycles <= s.hard_cycles_to_safe_point
```

Diagnostics: `SCHED-HardLatencyBelowInterruptLatency`,
`SCHED-SoftTargetExceedsHardBound`.

#### 8.4.5 Lease yield-safety table

For every lease `l`, `l.yield_safe` is determined by the kind × yield
boundary table:

```text
F-LeaseYieldSafe:
  ∀ l ∈ SchedulePack[mode].leases.
    l.yield_safe == match l.kind {
      RomWindow(_)                    => false,
      SramPage(_)                     => false,
      Overlay(o)                      => OverlayPlan.region(o)
                                          .eviction_policy.persists_across_yield,
      InterruptMask(Disabled)         => false,
      InterruptMask(ShortCriticalSection) => false,
      InterruptMask(Enabled)          => true,
    }
```

Diagnostic: `LEASE-YieldSafetyTableViolation`.

#### 8.4.6 Epoch coverage and disjointness

```text
F-EpochCoverage:
  ⋃ { e.slices | e ∈ SchedulePack[mode].epochs }
    == { s.id | s ∈ SchedulePack[mode].slices }

F-EpochDisjoint:
  ∀ e1, e2 ∈ SchedulePack[mode].epochs with e1.id ≠ e2.id.
    e1.slices ∩ e2.slices == ∅
```

Diagnostics: `SCHED-EpochCoverageGap`, `SCHED-EpochOverlap`.

#### 8.4.7 SchedulePack mode-equivalence invariants

```text
F-ModePackKeysEqual:
  SchedulePack.modes.keys() == SchedulePack.epochs.keys()
                            == SchedulePack.leases.keys()

F-ModePackCheckpointSchema:
  ∀ mode ∈ SchedulePack.modes.keys().
    Hash256(SchedulePack.modes[mode].checkpoint_schema)
      == SchedulePack.checkpoint_schema_hash

F-ModePackContinuationAbi:
  ∀ mode ∈ SchedulePack.modes.keys().
    Hash256(continuation_abi_shape(SchedulePack.modes[mode]))
      == SchedulePack.continuation_abi_hash

F-ModePackEntryAlignment:
  ∀ mode ∈ SchedulePack.modes.keys().
    SchedulePack.modes[mode].entry_slice exists in
      SchedulePack.modes[mode].slices

F-ModePackNonEmpty:
  SchedulePack.modes.len() >= 1
```

Diagnostics: `MODE-KeysMismatch`, `MODE-CheckpointSchemaMismatch`,
`MODE-ContinuationAbiMismatch`, `MODE-EntrySliceMissing`,
`MODE-PackEmpty`.

#### 8.4.8 ModeSwitchPolicy validity

```text
F-ModeSwitchLegalPoints:
  SchedulePack.switch_policy.legal_switch_points ⊆
    ObservationPlan.semantic_checkpoint_pins.keys()

F-ModeSwitchLegalEpochBoundaries:
  ∀ b ∈ SchedulePack.switch_policy.legal_epoch_boundaries.
    ∃ mode ∈ SchedulePack.modes.keys(), e ∈ SchedulePack.epochs[mode].
      e.id == b
```

Diagnostics: `MODE-SwitchPointNotInObservationPlan`,
`MODE-SwitchEpochBoundaryMissing`.

#### 8.4.9 Drift envelope validity

```text
F-DriftEnvelopeV1Minimum:
  SchedulePack.drift_monitor.expected.slice_cycles_p95.is_some()

F-DriftMetricInClosedSet:
  ∀ t ∈ SchedulePack.switch_policy.drift_triggers.
    t.metric ∈ closed_DriftMetric_set()
    ∧ t.action ∈ closed_DriftAction_set()
```

Diagnostics: `DRIFT-V1MinimumViolated`,
`DRIFT-MetricNotInClosedSet`, `DRIFT-ActionNotInClosedSet`.

### 8.5 Canonical reference semantics — small-step semantics for slice execution and lease state

This subsection pins a canonical small-step semantics for `SchedSlice`
execution and lease state. The semantics is what `ResourceStateValidation`
(§9) interprets symbolically; it is also what F-C3 (`ScheduleOracle`)
binds emulator state to op-by-op.

#### 8.5.1 Machine state

The schedule-level machine state for proof purposes is:

```text
M := (slice_pc, lease_state, residency_state, scratch_state,
      yield_pending, isr_active)

slice_pc        : SliceId × OpIndex      -- which op of which slice
lease_state     : { LeaseId -> LeaseStatus }
                  LeaseStatus ∈ {Free, Held(SliceId)}
residency_state : (RomWindowBinding, Option<OverlayId>, Residency)
scratch_state   : opaque (F-B15 register file)
yield_pending   : bool
isr_active      : bool
```

#### 8.5.2 Step relation

The step relation `M -[op]-> M'` is defined per op:

```text
AcquireLease { lease } :
  pre:   lease_state[lease] == Free
         ∧ kind-specific consistency (e.g. RomWindow lease's bank id
            matches residency_state.rom_window.bank_id once held)
  post:  lease_state[lease] == Held(current_slice)

ReleaseLease { lease } :
  pre:   lease_state[lease] == Held(current_slice)
  post:  lease_state[lease] == Free

OverlayInstall { install } :
  pre:   surrounding lease scope satisfies OverlayLeaseShape(install)
  post:  residency_state.overlay = Some(install.region)
         ∧ residency-state-class transition to the new overlay member

BankSwitch { from, to } :
  pre:   ∃ release_event in op stream releasing RomWindow(from)
            and acquire_event in op stream acquiring RomWindow(to),
         both in the surrounding slice scope.
  post:  residency_state.rom_window.bank_id = to

KernelCall { spec, tile_index } :
  pre:   residency_state matches spec.required_residency
         ∧ scratch_state has spec.required_scratch live
  post:  scratch_state advanced by spec's effect

Load { value, src, dst } :
  pre:   src ∈ slice.live_wram ∪ slice.live_sram
  post:  scratch_state[dst] = ArenaPlan.lookup(src).value

Store { value, src, dst } :
  pre:   dst ∈ slice.live_wram ∪ slice.live_sram
  post:  ArenaPlan.lookup(dst).value = scratch_state[src]

Effect { effect } :
  pre:   effect ∈ slice.semantic_effect_set
  post:  effect-specific state advance (sequence-state mutation, RNG)

TraceProbe { probe } :
  pre:   probe ∈ ObservationPlan.trace_probe_pins
  post:  trace ring buffer advance (no slice-state effect)

SemanticCheckpoint { checkpoint } :
  pre:   checkpoint ∈ ObservationPlan.semantic_checkpoint_pins
  post:  checkpoint id recorded for ScheduleOracle consumption

PersistCommit { group } :
  pre:   slice.exit_kind ∈ {SaveContinuationAndYield (with Yield kind ∈
                                 {NeedInput, TokenReady, Finished})}
         ∨ explicit commit boundary declared in StoragePlan
  post:  Persistent record protocol: all pages in commit_group transition
         from Writing -> Committed

Yield { kind } :
  pre:   ∀ l ∈ outstanding leases. l.yield_safe == true
  post:  yield_pending = true; control transfers to scheduler

TailCall { target } :
  pre:   target.entry_residency == current residency_state
         ∧ outstanding leases at exit == target.required_leases
  post:  slice_pc = (target, 0)

EnterIsr { vector } :
  pre:   isr_active == false
         ∧ residency_state.rom_window matches Bank0
         ∧ ISR-target code is Bank0/HRAM/fixed-WRAM only
  post:  isr_active = true; scheduler dispatches ISR vector

Halt :
  post:  build complete

Fault { code } :
  post:  fault path entered; subsequent slice executes fault handler
```

The pre-conditions are the proof obligations Stage 10.5 discharges
symbolically. They are not runtime assertions; they are typed lemmas
proven by lease-flow analysis.

#### 8.5.3 Slice transition relation

Inter-slice transitions are governed by `ExitKind`:

```text
ExitKind::SaveContinuationAndYield :
  prev_slice.terminal_op = Yield { kind }
  next_slice ∈ prev_slice.successors
  yield_pending = true at the boundary
  scheduler decides next_slice based on:
    - yield kind (Micro -> immediate resume; Frame -> after frame
      service; NeedInput -> after input; TokenReady -> after commit;
      Finished -> halt; Fault -> fault path)
    - drift envelope state
    - mode switch policy

ExitKind::TailCall :
  prev_slice.terminal_op = TailCall { target }
  next_slice = target
  no scheduler dispatch

ExitKind::EnterIsr :
  prev_slice.terminal_op = EnterIsr { vector }
  next_slice = ISR-vector-specific handler slice
  isr_active = true

ExitKind::Halt :
  no successor

ExitKind::Fault :
  prev_slice.terminal_op = Fault { code }
  next_slice = fault-handler slice (one per build, in Bank0)
```

### 8.6 Op output value-format predicate

Storage class and lifetime are preserved from F-B8/F-B12 to Stage 10.
Specifically:

```text
F-StorageClassPreserved:
  ∀ Load { value, src, dst }, ∀ Store { value, src, dst } in any slice.
    let s = ArenaPlan.lookup(src or dst) in
    s.storage_class == StoragePlan.lookup(value).materialization.storage_class
    ∧ s.lifetime_class == StoragePlan.lookup(value).materialization.lifetime_class
    ∧ s.alias_class_id == StoragePlan.lookup(value).alias_class
```

Diagnostic: `SCHED-StorageClassDrifted`.

This means F-B13 cannot move a value to a different `StorageClass` (e.g.
demote `WramHot` to `SramPaged`) — that is the storage planner's job.
F-B13 reads the bindings and respects them.

## 9. Stage 10.5 contract: ResourceStateValidation

Stage 10.5 is the typed proof that a `SchedulePack` produced by Stage 10
is interrupt-safe, lease-balanced, residency-correct, and overlay/bank-
shadow consistent. It runs as a separate pass with a separate
StageCache key and emits `certs/resource_state.cert.json`.

### 9.1 Proof obligations

The certificate discharges four classes of proof obligation. Each is a
typed predicate over `SchedulePack`.

#### 9.1.1 Lease balance

**Statement.** Every `AcquireLease` op has a matching `ReleaseLease` op
on every reachable slice path; no lease is acquired twice without an
intervening release; no lease is released without being held.

**Formal predicate.** Let `G` be the slice graph (nodes are slices,
edges are successor relations and tail-call edges). Let `paths(s)` be
the set of paths through `G` from the entry slice to slice `s`. For
every lease `l`:

```text
F-LeaseBalance:
  ∀ l ∈ SchedulePack[mode].leases.
    ∀ s ∈ SchedulePack[mode].slices, ∀ p ∈ paths(s).
      let acquire_count(l, p) = |{ op ∈ ops(p) | op = AcquireLease(l) }|
      let release_count(l, p) = |{ op ∈ ops(p) | op = ReleaseLease(l) }|
      acquire_count(l, p) ∈ {0, 1}
      release_count(l, p) ∈ {0, 1}
      acquire_count(l, p) >= release_count(l, p)
      (i.e. every prefix has at least as many acquires as releases)

  ∀ l ∈ SchedulePack[mode].leases.
    ∀ p ∈ all_paths_to_terminal_slices.
      acquire_count(l, p) == release_count(l, p)
      (i.e. every terminal-reaching path has matching counts)
```

**Decision procedure.** §9.3.1.

**Diagnostic codes.** `LEASE-Unbalanced { lease, path }`,
`LEASE-DoubleAcquire { lease, path }`, `LEASE-ReleaseWithoutAcquire { lease, path }`.

**Certificate evidence.** Per-lease per-path acquire/release counts
recorded as a checked fact: `LeaseBalanceFact { lease, slice_path,
acquire_count, release_count }`.

#### 9.1.2 Yield-safety (no illegal yield crosses a non-resumable lease)

**Statement.** No `Yield`-class transition crosses a lease whose
`yield_safe = false`.

**Formal predicate.**

```text
F-YieldSafety:
  ∀ s ∈ SchedulePack[mode].slices with s.exit_kind = SaveContinuationAndYield.
    let outstanding_at_exit(s) = { l ∈ leases | l.acquired_in dominates s
                                              ∧ l.released_in does not dominate s
                                              ∧ s in surrounding scope }
    ∀ l ∈ outstanding_at_exit(s). l.yield_safe == true
```

Equivalently: the set of leases held at any `Yield` op contains only
yield-safe leases.

**Decision procedure.** §9.3.2.

**Diagnostic codes.** `LEASE-YieldCrossesNonResumable { lease, slice }`.

**Certificate evidence.** Per-yield-event lease snapshot: `YieldSafetyFact {
slice, yield_kind, outstanding_leases, all_yield_safe }`.

#### 9.1.3 ISR-visible-residency (no ISR-visible path depends on leased switchable state)

**Statement.** No slice marked `interrupt_policy = Enabled` (and no slice
reachable from it within an interrupt-enabled window) depends on leased
switchable state — i.e. while interrupts are enabled, no `RomWindow` or
`SramPage` lease is held by any code path that an ISR vector could
preempt and interleave with.

**Formal predicate (annotation-driven, v1).**

```text
F-IsrVisibleResidency:
  ∀ s ∈ SchedulePack[mode].slices with s.interrupt_policy = Enabled.
    let outstanding(s) = leases held while executing any op of s
    ∀ l ∈ outstanding(s).
      l.kind ∉ { RomWindow(_), SramPage(_) }
      ∨ l.yield_safe == true   -- (vacuously false for RomWindow/SramPage; see §8.4.5)

  ∀ s ∈ SchedulePack[mode].slices with s.interrupt_policy = Enabled.
    s.entry_residency = Bank0 ∨ s.entry_residency = Common(_)
    -- ISR-visible code may not execute from Expert(_) banks
```

**Equivalently.** If a slice has `interrupt_policy = Enabled`, the slice
must execute from Bank0 or a Common bank (not from an Expert bank), and
the slice must not hold a `RomWindow` or `SramPage` lease across any of
its ops.

**Decision procedure.** §9.3.3.

**Diagnostic codes.** `RES-IsrEnabledHoldsRomWindowLease { slice, lease }`,
`RES-IsrEnabledHoldsSramPageLease { slice, lease }`,
`RES-IsrEnabledInExpertBank { slice, residency }`.

**Certificate evidence.** Per-`Enabled` slice fact:
`IsrVisibleResidencyFact { slice, residency, outstanding_leases,
all_safe }`.

**Caveat.** This is annotation-driven; F-B15's `ReachabilityValidation`
is the *computed* version that catches violations emerging from
far-call legalization. See §1.7 and §9.4.

#### 9.1.4 Overlay/bank-shadow consistency

**Statement.** Every slice's `entry_residency` matches the
`ResidencyEpoch` it belongs to; every `OverlayInstall` referenced by a
lease is a member of the declared `ResidencyEpoch`'s overlay set; every
`BankSwitch` op is bracketed by matching lease release+acquire events.

**Formal predicate.**

```text
F-OverlayBankShadowConsistency:
  ∀ s ∈ SchedulePack[mode].slices.
    let e = SchedulePack[mode].epochs.find(|e| s.id ∈ e.slices)
    s.entry_residency == e.residency
    ∧ ∀ op ∈ s.ops with op = OverlayInstall(install).
        OverlayPlan.lookup(install).region == e.overlay.unwrap_or(_)
        -- the overlay being installed matches the epoch's overlay slot
    ∧ ∀ op ∈ s.ops with op = BankSwitch { from, to }.
        ∃ release_event before op : ReleaseLease(l) where
            l.kind == RomWindow(_) ∧ l.kind.bank_id == from
        ∧ ∃ acquire_event before op : AcquireLease(l') where
            l'.kind == RomWindow(_) ∧ l'.kind.bank_id == to
```

**Decision procedure.** §9.3.4.

**Diagnostic codes.** `RES-EntryResidencyEpochMismatch { slice, epoch }`,
`RES-OverlayInstallEpochMismatch { install, epoch }`,
`RES-BankSwitchUnbracketed { slice, op_index, from, to }`.

**Certificate evidence.** Per-slice/per-op fact:
`OverlayBankShadowConsistencyFact { slice, epoch_id,
overlay_install_alignment, bank_switch_bracketing }`.

### 9.2 Certificate shape

The certificate is `certs/resource_state.cert.json`, schema
`resource_state.cert.v1`.

```rust
pub struct ResourceStateCertBody {
    pub identity: ResourceStateIdentitySection,
    pub schedule_pack: SchedulePackIdentity,
    pub lease_balance: LeaseBalanceSection,
    pub yield_safety: YieldSafetySection,
    pub isr_visible_residency: IsrVisibleResidencySection,
    pub overlay_bank_shadow: OverlayBankShadowSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct ResourceStateIdentitySection {
    pub sched_ir_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub arena_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub quant_graph_self_hash: Hash256,
}

pub struct SchedulePackIdentity {
    pub modes: Vec<RuntimeMode>,
    pub checkpoint_schema_hash: Hash256,
    pub continuation_abi_hash: Hash256,
    pub mode_switch_policy_hash: Hash256,
    pub drift_monitor_hash: Hash256,
}

pub struct LeaseBalanceSection {
    pub leases: Vec<LeaseBalanceFact>,
    pub all_balanced: bool,
}

pub struct LeaseBalanceFact {
    pub lease: LeaseId,
    pub kind_discriminant: ResourceLeaseKindDiscriminant,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
    pub paths_checked: u32,
    pub all_paths_balanced: bool,
}

pub struct YieldSafetySection {
    pub yield_events: Vec<YieldSafetyFact>,
    pub all_yields_safe: bool,
}

pub struct YieldSafetyFact {
    pub slice: SliceId,
    pub yield_kind: YieldKind,
    pub outstanding_leases: Vec<LeaseId>,
    pub all_yield_safe: bool,
}

pub struct IsrVisibleResidencySection {
    pub enabled_slices: Vec<IsrVisibleResidencyFact>,
    pub all_isr_safe: bool,
    /// True iff F-B15 ReachabilityValidation has independently confirmed
    /// the annotated residency at the computed-reachability level. v1
    /// always sets this to false; F-B15's pass updates the certificate
    /// or emits a separate certs/reachability.cert.json that supersedes
    /// the annotation-driven evidence.
    pub computed_reachability_confirmed: bool,
}

pub struct IsrVisibleResidencyFact {
    pub slice: SliceId,
    pub residency: Residency,
    pub outstanding_leases: Vec<LeaseId>,
    pub all_safe: bool,
}

pub struct OverlayBankShadowSection {
    pub slices_checked: Vec<OverlayBankShadowConsistencyFact>,
    pub all_consistent: bool,
}

pub struct OverlayBankShadowConsistencyFact {
    pub slice: SliceId,
    pub epoch_id: EpochId,
    pub entry_residency_matches: bool,
    pub overlay_installs_aligned: bool,
    pub bank_switches_bracketed: bool,
    pub all_consistent: bool,
}
```

**Outcome semantics:**

* `outcome = Passed` iff `lease_balance.all_balanced ∧
  yield_safety.all_yields_safe ∧ isr_visible_residency.all_isr_safe ∧
  overlay_bank_shadow.all_consistent ∧ diagnostics.is_empty()`.
* `outcome = Failed` iff at least one section has a violation; at
  least one `Hard` diagnostic is recorded.
* No `Soft` diagnostic in this chunk (§7).

**Field invariants:**

* `LeaseBalanceFact` for every `LeaseId` in `SchedulePack.leases.values()`.
* `YieldSafetyFact` for every slice with `exit_kind = SaveContinuationAndYield`.
* `IsrVisibleResidencyFact` for every slice with `interrupt_policy = Enabled`.
* `OverlayBankShadowConsistencyFact` for every slice in
  `SchedulePack.modes[mode].slices` for every `mode`.

### 9.3 Decision procedure

The decision procedure is **typed lease-flow analysis**, not a runtime
simulation. It is a symbolic computation over the slice graph and the
op sequence within each slice.

#### 9.3.1 Lease-flow analysis (lease balance)

For each lease `l`, perform a flow-graph traversal of the slice graph
starting at the entry slice. At each slice, walk the op sequence and
maintain a per-path counter of `(acquire_count, release_count)`. At
slice boundaries, propagate counters along successor edges.

```text
algorithm lease_balance(SchedulePack, LeaseId l) -> Result:
  let G = slice_graph(SchedulePack)
  let entry = SchedulePack.modes[mode].entry_slice
  let visited = {}
  let counters = {entry: (0, 0)}
  let queue = [entry]
  let facts = []

  while not queue.empty():
    let s = queue.pop()
    if s in visited: continue
    visited.insert(s)

    let (acq, rel) = counters[s]
    for op in slice(s).ops:
      if op = AcquireLease(l): acq += 1
      if op = ReleaseLease(l): rel += 1
      if rel > acq:
        return Err(LEASE-ReleaseWithoutAcquire { lease: l, slice: s })

    if slice(s).exit_kind ∈ {Halt, Fault}:
      if acq != rel:
        return Err(LEASE-Unbalanced { lease: l, terminal_slice: s,
                                       acq_count: acq, rel_count: rel })
    else:
      for s' in slice(s).successors:
        if s' in counters and counters[s'] != (acq, rel):
          return Err(LEASE-PathDivergent { lease: l, joining_slice: s' })
        counters[s'] = (acq, rel)
        queue.push(s')

    facts.push(LeaseBalanceFact {
      lease: l,
      kind_discriminant: l.kind.discriminant(),
      acquired_in: l.acquired_in,
      released_in: l.released_in,
      yield_safe: l.yield_safe,
      paths_checked: visited.size,
      all_paths_balanced: true,
    })

  return Ok(facts)
```

**Termination.** The slice graph is a DAG over slices in any one mode.
The algorithm visits each slice at most once per lease.

**Correctness.** The algorithm establishes that every path from the
entry slice to any terminal slice has equal acquire and release counts.
This is the formal predicate F-LeaseBalance.

**Determinism.** The algorithm is deterministic: queue order is fixed
(BFS by slice id ascending); successor order is fixed (sort by slice
id ascending).

#### 9.3.2 Yield-safety analysis

For each slice with `exit_kind = SaveContinuationAndYield`, compute the
set of leases held at the slice's terminal `Yield` op:

```text
algorithm yield_safety(SchedulePack, SliceId s) -> YieldSafetyFact:
  let outstanding = compute_outstanding_leases_at_op(s, terminal_yield_op)
  let all_yield_safe = ∀ l ∈ outstanding. l.yield_safe == true
  return YieldSafetyFact {
    slice: s,
    yield_kind: terminal_yield_op.kind,
    outstanding_leases: outstanding,
    all_yield_safe,
  }

algorithm compute_outstanding_leases_at_op(s: SliceId, op_idx: usize)
                                          -> Set<LeaseId>:
  -- Compute the lease holds-set at op_idx of slice s by:
  --   1. taking the lease state at slice entry (from path analysis);
  --   2. applying acquire/release ops in s.ops[..op_idx] sequentially.
  -- The lease state at entry is well-defined because lease balance
  -- (§9.3.1) ensures all paths to s agree on the lease state.
```

**Correctness.** The lease holds-set is well-defined because §9.3.1
established path agreement.

#### 9.3.3 ISR-visible residency analysis

For each slice with `interrupt_policy = Enabled`:

```text
algorithm isr_visible_residency(SchedulePack, SliceId s)
                                -> IsrVisibleResidencyFact:
  -- For each op in slice s, compute outstanding leases.
  -- Verify no RomWindow or SramPage lease is in the outstanding set.
  -- Verify s.entry_residency ∈ {Bank0, Common(_)}.

  let all_safe = true
  let outstanding_at_any_op = []

  for op_idx in 0..s.ops.len():
    let outstanding = compute_outstanding_leases_at_op(s, op_idx)
    outstanding_at_any_op.extend(outstanding)
    for l in outstanding:
      if l.kind ∈ {RomWindow(_), SramPage(_)}:
        all_safe = false
        emit Diagnostic(RES-IsrEnabledHoldsRomWindowLease or
                          RES-IsrEnabledHoldsSramPageLease,
                        slice: s, lease: l)

  if s.entry_residency ∉ {Bank0, Common(_)}:
    all_safe = false
    emit Diagnostic(RES-IsrEnabledInExpertBank,
                    slice: s, residency: s.entry_residency)

  return IsrVisibleResidencyFact {
    slice: s,
    residency: s.entry_residency,
    outstanding_leases: deduped(outstanding_at_any_op),
    all_safe,
  }
```

**Correctness.** Equivalent to F-IsrVisibleResidency by construction.

**Caveat.** This is annotation-driven (§1.7). The computed counterpart
in F-B15 sees the full transitive reachability after far-call
legalization.

#### 9.3.4 Overlay/bank-shadow consistency

For each slice and each op in the slice:

```text
algorithm overlay_bank_shadow(SchedulePack, SliceId s)
                              -> OverlayBankShadowConsistencyFact:
  let e = SchedulePack.epochs.find(|e| s.id ∈ e.slices)
  let entry_residency_matches = (s.entry_residency == e.residency)

  let overlay_installs_aligned = true
  for op in s.ops:
    if op = OverlayInstall(install):
      let region = OverlayPlan.lookup(install).region
      if Some(region) != e.overlay:
        overlay_installs_aligned = false

  let bank_switches_bracketed = true
  for op_idx, op in s.ops.iter().enumerate():
    if op = BankSwitch { from, to }:
      let outstanding_before = compute_outstanding_leases_at_op(s, op_idx)
      let from_released = ∃ op' = ReleaseLease(l) at idx < op_idx in s.ops
                            where l.kind == RomWindow(_)
                                  ∧ l.kind.bank_id == from
      let to_acquired = ∃ op' = AcquireLease(l) at idx < op_idx in s.ops
                          where l.kind == RomWindow(_)
                                ∧ l.kind.bank_id == to
      if not (from_released ∧ to_acquired):
        bank_switches_bracketed = false

  return OverlayBankShadowConsistencyFact {
    slice: s,
    epoch_id: e.id,
    entry_residency_matches,
    overlay_installs_aligned,
    bank_switches_bracketed,
    all_consistent: entry_residency_matches ∧ overlay_installs_aligned
                  ∧ bank_switches_bracketed,
  }
```

**Correctness.** Equivalent to F-OverlayBankShadowConsistency.

### 9.4 Semantics of "ISR-visible" before F-B15 ReachabilityValidation lands

The ISR-visible-residency check (§9.1.3) has two regimes:

* **v1 (this RFC, annotation-driven).** Stage 10.5 trusts the slice's
  declared `interrupt_policy`, `entry_residency`, and lease set. It
  catches violations that are visible at the schedule level: an
  `Enabled` slice in an Expert bank, an `Enabled` slice holding a
  `RomWindow` lease, etc.
* **post-F-B15 (computed).** F-B15's `ReachabilityValidation` pass
  computes the transitive reachability classes after far-call
  legalization. It catches violations that emerge from far-call thunks,
  jump tables, or interrupt-vector dispatch tables that span more than
  one bank.

The `ResourceStateCertBody.isr_visible_residency.computed_reachability_confirmed`
field is `false` in v1. F-B15's pass either:

* updates the existing `resource_state.cert.json` to set
  `computed_reachability_confirmed = true` and emit additional
  evidence; or
* emits a separate `certs/reachability.cert.json` that supersedes the
  annotation-driven evidence.

For v1 closure, the annotation-driven evidence is sufficient. F-B15's
later confirmation is part of the M2/M3 backend work.

**Why annotation-driven now:** because Stage 10 hasn't yet emitted
AsmIR or far-call thunks. The annotation-driven check is the upper
bound on what is locally provable. It is a real proof — every
annotated violation is a real violation — but it is not the *complete*
proof. F-B15 closes the gap.

**Why two passes and not one:** F-B15 needs the schedule to be in tree
to run its analysis. F-B13 needs to fail fast on schedule-level
violations before AsmIR exists. Two passes is the minimum.

## 10. SchedulePack multi-mode semantics

### 10.1 Mode-equivalence invariants

Modes share artifact semantics. The invariants pinned by §8.4.7 ensure
that a `SchedulePack` with multiple modes is semantically equivalent
across modes — the modes differ only in *how* the same compute is
scheduled, not *what* is computed.

```text
F-ModeSemanticEquivalence:
  ∀ mode_a, mode_b ∈ SchedulePack.modes.keys().
    SchedulePack.checkpoint_schema_hash is shared
    SchedulePack.continuation_abi_hash is shared
    The set of SemanticCheckpointId pins emitted by each mode is equal
      (same observation contract; different ops on the path between
       checkpoints).
    The set of EffectId edges traversed in topological order is equal
      (same effect linearization; different scheduling).
    The final continuation state at any matching SemanticCheckpointId
      is equal modulo the artifact's DeterminismClass.
```

### 10.2 Legal switch points

A mode switch may only occur at a `SemanticCheckpointId` declared in
`SchedulePack.switch_policy.legal_switch_points` and at an
`EpochId` declared in `legal_epoch_boundaries`. The intersection of
those constraints is the set of moments at which the runtime may
transition from mode A to mode B.

```text
F-ModeSwitchLegalMoment:
  ∀ switch event (mode_a -> mode_b) at moment m.
    m.checkpoint ∈ SchedulePack.switch_policy.legal_switch_points
    ∧ m.epoch ∈ SchedulePack.switch_policy.legal_epoch_boundaries
    ∧ continuation_state_at(m, mode_a)
       == continuation_state_at(m, mode_b)
    ∧ residency_state_at(m, mode_a) is reachable to
        residency_state_at(m, mode_b) via the same legal lease/install
        events as a single-mode transition.
```

The runtime side (F-D6) executes the actual switch — saves continuation
state in the canonical layout, swaps the active `GbSchedIR` reference,
restores state in the new mode's slice context.

### 10.3 Continuation ABI parity

Every mode's `GbSchedIR` shares one continuation ABI:

* `gbf-abi::InferenceState` shape (the `cont_*` fields, slice id width,
  arena cursor type).
* `gbf-abi::HarnessCommandBlock` and `HarnessResultBlock` shapes.
* Slice id allocation pattern: ids are allocated globally across modes
  to a single id space, so a slice id from mode A is also a valid
  reference from mode B's slice graph (though not necessarily a valid
  successor).

```text
F-ContinuationAbiParity:
  ∀ mode_a, mode_b ∈ SchedulePack.modes.keys().
    SizeOf(InferenceState[mode_a]) == SizeOf(InferenceState[mode_b])
    ∧ alignment(InferenceState[mode_a]) == alignment(InferenceState[mode_b])
    ∧ field layout of InferenceState across modes is identical.
```

The runtime can save state in mode A and restore in mode B, modulo a
one-time slice-id translation table maintained by F-D6.

### 10.4 Mode requirement vs mode emission

`ResolvedCompilePolicy.requested_runtime_modes` declares which modes the
build must support. F-B13 emits a `SchedulePack` whose `modes.keys()`
is exactly that set — no fewer (would fail the policy check), no more
(would inflate artifact bytes).

```text
F-ModeKeysMatchPolicy:
  SchedulePack.modes.keys() == ResolvedCompilePolicy.requested_runtime_modes
```

**Default profile.** v1 default profile requests
`{ SteadyStateGeneration }`. v1 bringup profile requests
`{ InteractiveTyping, SteadyStateGeneration }`. v1 trace profile requests
all three (`{ InteractiveTyping, SteadyStateGeneration,
TraceHeavyDebugging }`). v1 recovery profile requests `{ SafeMode }`.

### 10.5 Mode switch v1 closure surface

For v1 closure, mode switching is **schema-only**. The runtime executes
exactly one mode for the lifetime of one `gbf-codegen` invocation;
mode-switching producers/consumers live in F-D6 (BLOCKED on oracle).

The chunk-closure fixture exercises:

* a single-mode build (`SchedulePack.modes.len() == 1`);
* a multi-mode build (`SchedulePack.modes.len() == 3`) with
  `legal_switch_points.len() == 0` (no legal switches; the multi-mode
  artifact is valid but cannot switch yet);
* a multi-mode build with at least one legal switch point that matches
  a `SemanticCheckpointId` pin in `ObservationPlan`.

The third case exercises the schema; F-D6 lands the runtime side.

## 11. Drift monitor contract

### 11.1 Producer/consumer split

The drift monitor is a structured handshake between compiler and
runtime:

```text
compiler (F-B13)         runtime (F-D1)
  produces:                produces:
    expected envelope        observed envelope
    drift triggers           consecutive_violations counter
    safe-mode triggers       drift action dispatch
    mode-switch policy
```

The compiler-side `expected` envelope is bound to compiled choices
(slice budgets, tile sizes, observation density). The runtime-side
`observed` envelope is the actual measurements over a sliding window
(`window_frames`).

### 11.2 Expected envelope derivation

`SchedulePack.drift_monitor.expected.slice_cycles_p95` is computed from
the per-slice `soft_target_cycles` distribution:

```text
expected.slice_cycles_p95 = quantile_p95({ s.soft_target_cycles |
                                           s ∈ SchedulePack.modes[default].slices })
```

The other three fields are optional in v1:

* `ui_commit_cycles_p95` — Some only if the `ObservationPlan.ui_pressure_pins`
  set is non-empty.
* `trace_drop_rate_pct` — Some only if `ObservabilityMode != Off`.
* `persist_overrun_rate_pct` — Some only if any `Persist` materialization
  exists in `StoragePlan`.

```text
F-ExpectedEnvelopeDerivation:
  expected.slice_cycles_p95 == quantile_p95(slice soft targets)
  expected.ui_commit_cycles_p95 ∈ {None,
       quantile_p95(ui_commit_pressure projections from F-B6)}
  expected.trace_drop_rate_pct ∈ {None,
       trace_drop estimate from ObservationPlan trace_budget}
  expected.persist_overrun_rate_pct ∈ {None,
       persist overrun estimate from SramPagePlan commit_boundaries}
```

### 11.3 Drift triggers

Each `DriftTrigger` declares an action when the metric exceeds the
threshold for `consecutive_violations >= consecutive_violation_floor`
windows.

```text
DriftTrigger {
  metric: SliceCyclesP95,
  threshold: expected.slice_cycles_p95.unwrap() * 12 / 10,  -- +20%
  action: DriftAction::ShrinkSlices,
}

DriftTrigger {
  metric: TraceDropRatePct,
  threshold: 5,  -- 5% drop rate
  action: DriftAction::DropTrace,
}

DriftTrigger {
  metric: UiCommitCyclesP95,
  threshold: ...,
  action: DriftAction::DemoteMode(RuntimeMode::InteractiveTyping),
}
```

**v1 default trigger set:** the default profile ships:

* `(SliceCyclesP95, 1.2 × expected, ShrinkSlices)`;
* `(UiCommitCyclesP95, 1.5 × expected, DemoteMode(InteractiveTyping))`
  if the build requests `InteractiveTyping`.

Other triggers are emitted only if the corresponding
`expected.<metric>` field is `Some`.

### 11.4 Drift action semantics

* **`ShrinkSlices`** — runtime shrinks the effective slice size (yields
  earlier than the compile-time soft target). Implementation: the
  scheduler uses `hard_cycles_to_safe_point` as a hard ceiling and
  picks earlier yield points within the slice's `YieldCheckClass`
  pattern. No new slices are minted at runtime; the existing slice
  graph is honored.
* **`DropTrace`** — runtime drops trace probes when the trace ring is
  near full. `TraceProbe` ops become no-ops; the trace-drop counter
  advances.
* **`DemoteMode(mode)`** — runtime switches active mode at the next
  `legal_switch_point`. If the demoted mode is not in
  `SchedulePack.modes.keys()`, the runtime falls back to the closest
  available mode (typically `SafeMode`).

### 11.5 Window and consecutive-violation semantics

```text
RuntimeDriftMonitor.window_frames = N (typically 60 = 1 second @60fps)
RuntimeDriftMonitor.consecutive_violations: u8 (saturating)
```

The runtime measures `observed` over the last N frames. If any metric
exceeds its trigger threshold, `consecutive_violations` increments. If
no metric exceeds, `consecutive_violations` decays to 0 immediately.

When `consecutive_violations >= consecutive_violation_floor` (default
3), the trigger's action is dispatched. The action persists until the
metric falls below the threshold for at least `window_frames` frames.

## 12. Liveness contract

### 12.1 Liveness is not optional

A cooperative runtime can be locally safe and globally broken
(`planv0.md` line 2215). Liveness — the property that the inference
makes measurable progress over time — is part of the slice contract.

Every slice must make measurable progress at the
`SemanticCheckpointId` level. The compiler declares that a build's
`progress_epoch` advances at every `SemanticCheckpoint` op.

### 12.2 progress_epoch advancement

```text
F-ProgressEpochAdvancement:
  ∀ s ∈ SchedulePack[mode].slices.
    if s.semantic_checkpoint_pins.len() > 0:
      s.ops contains a SemanticCheckpoint op for each pin
      ∧ that SemanticCheckpoint op increments InferenceState.progress_epoch
```

The runtime checks `progress_epoch` advancement in
`gbf-abi::LivenessCounters` (per F-A3); if `no_progress_frames` exceeds
`max_no_progress_frames` (a `RuntimeChromeBudget` field), a
`LivenessTimeout` fault is raised, which is a `SafeModeTrigger`.

### 12.3 max_no_progress_frames

```text
RuntimeChromeBudget.max_no_progress_frames: u16  (default 600 = 10 seconds @60fps)
```

The compiler does not enforce this bound; it declares it. The runtime
enforces. When the runtime triggers `LivenessTimeout`, the cooperative
scheduler transitions to `SafeMode` per `safe_mode_triggers`.

### 12.4 SafeMode triggers

```text
SafeModeTrigger ∈ {
  FaultClass(FaultClass),       -- specific fault classes
  LivenessTimeout,              -- max_no_progress_frames exceeded
  PersistentRecordCorruption,   -- persist record CRC failure
  HarnessAbort,                 -- harness commanded abort
}
```

Triggering any of these causes the runtime (F-D1) to:

* swap the active `GbSchedIR` to `RuntimeMode::SafeMode`'s schedule;
* flush the trace ring;
* mark the persist record set as recovery-pending;
* surface the fault code via `gbf-abi::FaultCode`.

For v1 closure, `SafeMode`'s `GbSchedIR` may be a minimal "halt-cleanly"
schedule that ends in `Halt` after writing a final `FaultCode`. F-D6
will extend this in M4+.

### 12.5 Liveness in the certificate

The certificate records the per-slice progress contribution:

```text
LivenessFact {
  slice: SliceId,
  progress_epoch_increments: u8,
  semantic_checkpoint_pins: Vec<SemanticCheckpointId>,
}

LivenessSection {
  slices: Vec<LivenessFact>,
  total_progress_per_token: u32,  -- sum of progress_epoch increments
  bound_against_max_no_progress: u16,  -- = max_no_progress_frames
}
```

The certificate's `LivenessSection.total_progress_per_token > 0` is a
required invariant: every token must produce at least one
`progress_epoch` increment.

```text
F-LivenessTotalProgress:
  certs/resource_state.cert.json.liveness.total_progress_per_token > 0
```

Diagnostic: `LIVENESS-NoProgressPerToken`.

## 13. Report schemas

### 13.1 sched_ir.json (sched_ir.v1)

`sched_ir.json` is the Stage 10 product report. It is a **full
SchedulePack snapshot** with every load-bearing field hashed and
recorded.

```rust
pub struct SchedIrReportBody {
    pub identity: SchedIrIdentitySection,
    pub schedule_pack: SchedulePackSection,
    pub mode_switch_policy: ModeSwitchPolicySection,
    pub drift_monitor: DriftMonitorSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct SchedIrIdentitySection {
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub arena_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
}

pub struct SchedulePackSection {
    pub modes: Vec<RuntimeMode>,
    pub checkpoint_schema_hash: Hash256,
    pub continuation_abi_hash: Hash256,
    pub per_mode: BTreeMap<RuntimeMode, PerModeSection>,
}

pub struct PerModeSection {
    pub mode: RuntimeMode,
    pub gb_sched_ir_hash: Hash256,
    pub slices: Vec<SliceSection>,
    pub epochs: Vec<EpochSection>,
    pub leases: Vec<LeaseSection>,
    pub entry_slice: SliceId,
    pub slice_count: u32,
    pub epoch_count: u32,
    pub lease_count: u32,
}

pub struct SliceSection {
    pub id: SliceId,
    pub op_count: u32,
    pub hard_cycles_to_safe_point: u32,
    pub soft_target_cycles: u32,
    pub max_interrupt_latency: u32,
    pub resources: ResourceVectorSection,
    pub live_wram_count: u16,
    pub live_sram_count: u16,
    pub yield_kind: YieldKind,
    pub yield_check: YieldCheckClass,
    pub entry_residency: ResidencySection,
    pub interrupt_policy: InterruptPolicy,
    pub required_lease_count: u16,
    pub exit_kind: ExitKind,
    pub semantic_checkpoint_pin_count: u16,
    pub trace_probe_pin_count: u16,
    pub successor_count: u16,
    pub op_sequence_hash: Hash256,
}

pub struct ResourceVectorSection {
    pub bank_switches: u16,
    pub sram_page_switches: u16,
    pub trace_bytes: u16,
    pub persist_bytes: u16,
    pub overlay_installs: u8,
}

pub struct ResidencySection {
    pub class: ResidencyClass,
    pub bank_id: Option<u16>,
    pub expert_id: Option<u16>,
}

pub enum ResidencyClass { Bank0, Common, Expert }

pub struct EpochSection {
    pub id: EpochId,
    pub rom_window_bank_id: u16,
    pub overlay: Option<OverlayId>,
    pub residency: ResidencySection,
    pub slice_count: u32,
}

pub struct LeaseSection {
    pub id: LeaseId,
    pub kind: LeaseKindSection,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
}

pub enum LeaseKindSection {
    RomWindow { bank_id: u16 },
    SramPage { page_id: u16 },
    Overlay { overlay_id: OverlayId },
    InterruptMask { policy: InterruptPolicy },
}

pub struct ModeSwitchPolicySection {
    pub legal_switch_points: Vec<SemanticCheckpointId>,
    pub legal_epoch_boundaries: Vec<EpochId>,
    pub ui_pressure_thresholds: Vec<UiPressureThresholdSection>,
    pub safe_mode_triggers: Vec<SafeModeTriggerSection>,
    pub drift_triggers: Vec<DriftTriggerSection>,
}

pub struct UiPressureThresholdSection {
    pub metric: UiPressureMetric,
    pub threshold: u32,
    pub action: ModeSwitchActionSection,
}

pub enum ModeSwitchActionSection {
    SwitchTo(RuntimeMode),
    Stay,
}

pub enum SafeModeTriggerSection {
    FaultClass { class: u8 },
    LivenessTimeout,
    PersistentRecordCorruption,
    HarnessAbort,
}

pub struct DriftTriggerSection {
    pub metric: DriftMetric,
    pub threshold: u32,
    pub action: DriftActionSection,
}

pub enum DriftActionSection {
    ShrinkSlices,
    DropTrace,
    DemoteMode { mode: RuntimeMode },
}

pub struct DriftMonitorSection {
    pub expected: DriftEnvelope,
    pub observed: DriftEnvelope,  -- always all-None at compile time
    pub consecutive_violations: u8,  -- always 0 at compile time
    pub window_frames: u16,
}
```

**Semantic invariants** (validated by
`SchedIrReportBody::validate_semantics`):

* `schema == "sched_ir.v1"`;
* `outcome == Failed` iff at least one `Hard` diagnostic is present;
* `outcome == Passed` implies no `Hard` diagnostics, all identity
  hashes are present, and `schedule_pack.modes.len() >= 1`;
* `schedule_pack.per_mode.keys() == schedule_pack.modes`;
* `drift_monitor.observed == DriftEnvelope::all_none()` at compile
  time (the runtime fills it in);
* `drift_monitor.consecutive_violations == 0` at compile time;
* `drift_monitor.expected.slice_cycles_p95.is_some()` (per F-DriftV1Minimum);
* every `LeaseSection.kind` corresponds to a member of
  `ResourceLeaseKind`;
* every `SliceSection.id` is unique within `per_mode[mode]`;
* every `EpochSection.id` is unique within `per_mode[mode]`;
* every `LeaseSection.id` is unique within `per_mode[mode]`;
* `slices` array is sorted by `SliceId` ascending;
* `epochs` array is sorted by `EpochId` ascending;
* `leases` array is sorted by `LeaseId` ascending;
* `legal_switch_points` and `legal_epoch_boundaries` are sorted ascending;
* `report_self_hash` round-trips per F-B2/F-B4 §2.4;
* every diagnostic has typed provenance;
* no diagnostic has severity `Soft`.

**Why hash-not-payload for slice ops:** slice op sequences are large and
high-cardinality; the JSON report stores `op_sequence_hash` (SHA-256
over the canonical-JSON-encoded op array) rather than the full ops.
The full `SchedSlice` shape is in the in-memory product
(`SchedulePack`); the report is a *summary* whose hash chain proves
content-addressing.

The full op sequence may optionally appear in `stages/sched_ir/` under
the `Trace` profile (per `planv0.md` §"Reports and artifacts"). v1
closure does not require the full op array in `sched_ir.json`.

### 13.2 slice_report.json (slice_report.v1)

`slice_report.json` is the per-slice histogram report. It is a separate
JSON document because consumers (dashboards, F-B14, F-B16) want
slice-shape statistics without the full SchedulePack.

```rust
pub struct SliceReportBody {
    pub identity: SliceReportIdentitySection,
    pub per_mode_histograms: BTreeMap<RuntimeMode, PerModeSliceHistogram>,
    pub global_histograms: GlobalSliceHistogram,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}

pub struct SliceReportIdentitySection {
    pub sched_ir_self_hash: Hash256,
    pub policy_resolution_self_hash: Hash256,
}

pub struct PerModeSliceHistogram {
    pub mode: RuntimeMode,
    pub slice_count: u32,
    pub epoch_count: u32,
    pub lease_count: u32,
    pub yield_kind_distribution: BTreeMap<YieldKind, u32>,
    pub interrupt_policy_distribution: BTreeMap<InterruptPolicy, u32>,
    pub exit_kind_distribution: BTreeMap<ExitKind, u32>,
    pub residency_distribution: BTreeMap<ResidencyClass, u32>,
    pub hard_cycles_histogram: HistogramSection,
    pub soft_target_cycles_histogram: HistogramSection,
    pub max_interrupt_latency_histogram: HistogramSection,
    pub bank_switches_per_slice_histogram: HistogramSection,
    pub overlay_installs_per_slice_histogram: HistogramSection,
}

pub struct HistogramSection {
    pub bucket_count: u8,
    pub bucket_boundaries: Vec<u32>,    -- length = bucket_count + 1
    pub bucket_counts: Vec<u32>,        -- length = bucket_count
    pub min: u32,
    pub max: u32,
    pub p50: u32,
    pub p95: u32,
    pub p99: u32,
    pub mean_x_1000: u32,  -- mean × 1000, integer; no floating point
}

pub struct GlobalSliceHistogram {
    pub mode_count: u8,
    pub total_slice_count: u32,
    pub total_epoch_count: u32,
    pub total_lease_count: u32,
    pub max_hard_cycles: u32,
    pub max_max_interrupt_latency: u32,
    pub bank_switches_per_token_estimate: u16,
    pub sram_page_switches_per_token_estimate: u16,
    pub overlay_installs_per_token_estimate: u8,
}
```

**Semantic invariants:**

* `schema == "slice_report.v1"`;
* `outcome == Passed` iff no `Hard` diagnostics and all per-mode
  histograms are non-empty;
* per-mode histogram counts agree with `sched_ir.json`'s
  `slice_count`/`epoch_count`/`lease_count`;
* histograms use **fixed bucket boundaries** declared in the schema
  (no floating-point bucket math); v1 buckets:
  * `hard_cycles_histogram`: `[0, 100, 500, 1000, 2000, 5000, 10000, u32::MAX]`
    (8 boundaries → 7 buckets);
  * `max_interrupt_latency_histogram`: `[0, 50, 100, 200, 500, 1000, u32::MAX]`
    (7 boundaries → 6 buckets);
  * `bank_switches_per_slice_histogram`: `[0, 1, 2, 4, 8, u16::MAX]`
    (6 boundaries → 5 buckets);
  * `overlay_installs_per_slice_histogram`: `[0, 1, 2, 4, u8::MAX]`
    (5 boundaries → 4 buckets);
* `mean_x_1000` is the integer mean × 1000 (no floating point);
* `report_self_hash` round-trips;
* no `Soft` diagnostic.

### 13.3 certs/resource_state.cert.json (resource_state.cert.v1)

Defined in §9.2. Re-listed here for completeness.

```rust
// (See §9.2 for full type definitions.)
pub struct ResourceStateCertBody {
    pub identity: ResourceStateIdentitySection,
    pub schedule_pack: SchedulePackIdentity,
    pub lease_balance: LeaseBalanceSection,
    pub yield_safety: YieldSafetySection,
    pub isr_visible_residency: IsrVisibleResidencySection,
    pub overlay_bank_shadow: OverlayBankShadowSection,
    pub liveness: LivenessSection,
    pub diagnostics: Vec<ValidationDiagnosticRecord>,
}
```

**Semantic invariants:**

* `schema == "resource_state.cert.v1"`;
* `outcome == Passed` iff every section reports `all_*: true`;
* every `LeaseId` in `SchedulePack.leases.values()` appears exactly
  once in `lease_balance.leases`;
* every slice with `exit_kind = SaveContinuationAndYield` appears
  exactly once in `yield_safety.yield_events`;
* every slice with `interrupt_policy = Enabled` appears exactly once
  in `isr_visible_residency.enabled_slices`;
* every slice in any mode appears exactly once in
  `overlay_bank_shadow.slices_checked`;
* `liveness.total_progress_per_token > 0` if `outcome == Passed`;
* `report_self_hash` round-trips;
* no `Soft` diagnostic.

### 13.4 Reportable nullability summary

Allowed nullable fields in the v1 reports:

`sched_ir.v1`:
```text
schedule_pack.per_mode[mode].epochs[i].overlay
schedule_pack.per_mode[mode].slices[i].entry_residency.bank_id
schedule_pack.per_mode[mode].slices[i].entry_residency.expert_id
drift_monitor.expected.slice_cycles_p95   -- but Some required for v1 closure
drift_monitor.expected.ui_commit_cycles_p95
drift_monitor.expected.trace_drop_rate_pct
drift_monitor.expected.persist_overrun_rate_pct
drift_monitor.observed.slice_cycles_p95   -- always None at compile time
drift_monitor.observed.ui_commit_cycles_p95
drift_monitor.observed.trace_drop_rate_pct
drift_monitor.observed.persist_overrun_rate_pct
```

`slice_report.v1`:
```text
(none — every histogram is required)
```

`resource_state.cert.v1`:
```text
(none — every fact field is required when its slice / lease is in scope)
```

## 14. StageCache algebra — Stage 10 + Stage 10.5 keys

### 14.1 K10 — SchedIrCacheKey

```rust
pub struct K10 {
    pub schema_id: &'static str,           -- "sched_ir.v1"
    pub schema_version: SemVer,
    pub policy_resolution_self_hash: Hash256,
    pub artifact_validation_self_hash: Hash256,
    pub infer_ir_self_hash: Hash256,
    pub observation_plan_self_hash: Hash256,
    pub range_plan_self_hash: Hash256,
    pub storage_plan_self_hash: Hash256,
    pub sram_page_plan_self_hash: Hash256,
    pub rom_window_plan_self_hash: Hash256,
    pub overlay_plan_self_hash: Hash256,
    pub arena_plan_self_hash: Hash256,
    pub runtime_chrome_budget_hash: Hash256,
    pub feature_set_hash: Hash256,        -- cargo features that affect
                                          -- code generation behavior
}

K10.canonical_bytes := CanonicalJson(K10)
K10.cache_key := DomainHash(
    "gbf-codegen", "StageCacheKey", "sched_ir.v1", "1.0.0",
    K10.canonical_bytes)
```

The cache value is the tuple
`(SchedulePack, ReportEnvelope<SchedIrReportBody>,
ReportEnvelope<SliceReportBody>)`.

### 14.2 K10.5 — ResourceStateCacheKey

```rust
pub struct K10_5 {
    pub schema_id: &'static str,           -- "resource_state.cert.v1"
    pub schema_version: SemVer,
    pub sched_ir_self_hash: Hash256,
    pub feature_set_hash: Hash256,
}

K10_5.canonical_bytes := CanonicalJson(K10_5)
K10_5.cache_key := DomainHash(
    "gbf-codegen", "StageCacheKey", "resource_state.cert.v1", "1.0.0",
    K10_5.canonical_bytes)
```

The cache value is `ReportEnvelope<ResourceStateCertBody>`.

### 14.3 Cache discipline

* Stage 10's success entry is keyed by K10. A K10 hit replays the full
  product (SchedulePack + sched_ir.json + slice_report.json).
* Stage 10.5's success entry is keyed by K10.5, which depends *only*
  on the Stage 10 product hash. A K10.5 hit replays the certificate.
* A K10 hit does **not** imply K10.5 may be skipped. The certificate
  is independently emitted; if K10.5 misses (e.g. on a fresh
  checkout after a feature-flag change), Stage 10.5 re-runs.
* Stage 10's failure memo is keyed by K10. A failure memo replays the
  failure report verbatim.
* Stage 10.5's failure memo is keyed by K10.5. A failure memo replays
  the failure certificate verbatim.

### 14.4 feature_set_hash

`feature_set_hash` is the canonical hash of the active cargo feature
set that affects code generation behavior. F-B17's StageCache
integration sweep is the load-bearing source of truth for this hash;
F-B13 uses it verbatim.

If F-B17 has not yet landed, F-B13 uses a placeholder feature set
`{}` (the empty set) and emits a warning that the cache may
over-share across feature configurations. This is acceptable for v1
closure because the v1 default profile uses one feature set.

## 15. Diagnostic algebra — SCHED-*, RES-*, LEASE-*, MODE-*, DRIFT-*, LIVENESS- codes

The diagnostic codes are partitioned by origin. Every code is a
variant of the closed `ValidationCode` enum (extended from F-B2/F-B4).

### 15.1 SCHED-* codes (Stage 10 self-consistency)

```text
SCHED-LiveSlotLifetimeMismatch {
  slice: SliceId,
  slot: ArenaSlotRef,
  slice_lifetime_class: LifetimeClass,
  slot_lifetime_class: LifetimeClass,
}

SCHED-EntryResidencyMismatch    -- subsumed by RES-EntryResidencyEpochMismatch

SCHED-HardLatencyBelowInterruptLatency {
  slice: SliceId,
  hard_cycles: u32,
  max_interrupt_latency: u32,
}

SCHED-SoftTargetExceedsHardBound {
  slice: SliceId,
  soft_target: u32,
  hard_bound: u32,
}

SCHED-EpochCoverageGap {
  slice: SliceId,
  -- slice not assigned to any ResidencyEpoch
}

SCHED-EpochOverlap {
  epoch_a: EpochId,
  epoch_b: EpochId,
  overlapping_slices: Vec<SliceId>,
}

SCHED-StorageClassDrifted {
  value: ValueId,
  storage_plan_class: StorageClass,
  arena_slot_class: StorageClass,
}

SCHED-OpSequenceMalformed {
  slice: SliceId,
  reason: OpSequenceMalformReason,
}

pub enum OpSequenceMalformReason {
  NoTerminalOp,
  MultipleTerminalOps,
  TerminalOpNotLast,
  PreOpAfterTileLoop,
  TileLoopOpAfterPostOp,
  AcquireAfterTerminal,
  ReleaseAfterTerminal,
}

SCHED-TerminalOpInconsistentWithExitKind {
  slice: SliceId,
  exit_kind: ExitKind,
  terminal_op: SchedOpDiscriminant,
}

SCHED-LoadStoreSlotNotInLiveSet {
  slice: SliceId,
  op_index: usize,
  slot: ArenaSlotRef,
  expected_set: LiveSetKind,  -- WramOrSram
}

SCHED-KernelCallResidencyMismatch {
  slice: SliceId,
  op_index: usize,
  kernel_required: KernelResidency,
  slice_entry_residency: Residency,
}

SCHED-OverlayInstallLeaseShapeUnsatisfied {
  slice: SliceId,
  install: InstallId,
  required_lease_shape: OverlayLeaseShape,
}

SCHED-TailCallEntryResidencyMismatch {
  source_slice: SliceId,
  target_slice: SliceId,
  source_exit_residency: Residency,
  target_entry_residency: Residency,
}

SCHED-TailCallLeaseSetMismatch {
  source_slice: SliceId,
  target_slice: SliceId,
  source_outstanding_at_exit: Vec<LeaseId>,
  target_required_leases: Vec<LeaseId>,
}

SCHED-EnterIsrInNonIsrSlice {
  slice: SliceId,
}

SCHED-IsrEntrySliceNotBank0 {
  slice: SliceId,
  entry_residency: Residency,
}

SCHED-FaultPathResidencyMismatch {
  slice: SliceId,
  declared_residency: Residency,
  -- For fault paths to be reachable from any slice, fault handler
  -- residency must be Bank0.
}

SCHED-CycleBudgetOverflow {
  slice: SliceId,
  field: CycleBudgetField,
  value: u64,  -- promoted to u64 to detect overflow
}

pub enum CycleBudgetField {
  HardCyclesToSafePoint,
  SoftTargetCycles,
  MaxInterruptLatency,
}
```

### 15.2 LEASE-* codes (lease balance and shape)

```text
LEASE-RequiredLeaseNotAcquired {
  slice: SliceId,
  required_lease: LeaseId,
  -- the lease in s.required_leases that is not held by any path to s
}

LEASE-YieldSafetyTableViolation {
  lease: LeaseId,
  declared_yield_safe: bool,
  expected_yield_safe: bool,  -- per §8.4.5 table
}

LEASE-Unbalanced {
  lease: LeaseId,
  terminal_slice: SliceId,
  acquire_count: u32,
  release_count: u32,
}

LEASE-DoubleAcquire {
  lease: LeaseId,
  first_slice: SliceId,
  first_op_index: usize,
  second_slice: SliceId,
  second_op_index: usize,
}

LEASE-ReleaseWithoutAcquire {
  lease: LeaseId,
  slice: SliceId,
  op_index: usize,
}

LEASE-PathDivergent {
  lease: LeaseId,
  joining_slice: SliceId,
  -- different slice paths to the same slice produce different
  -- (acquire_count, release_count) pairs.
}

LEASE-YieldCrossesNonResumable {
  slice: SliceId,
  yield_op_index: usize,
  outstanding_lease: LeaseId,
  outstanding_lease_yield_safe: false,
}

LEASE-AcquireReleaseScopeViolation {
  lease: LeaseId,
  -- e.g. acquired_in is not a dominator of released_in,
  --      or the acquire op of `lease` doesn't appear in `acquired_in.ops`.
  reason: AcquireReleaseScopeReason,
}

pub enum AcquireReleaseScopeReason {
  AcquireOpMissingFromAcquiredInSlice,
  ReleaseOpMissingFromReleasedInSlice,
  AcquiredInDoesNotDominateReleasedIn,
  AcquireAfterReleaseInSameSlice,
}

LEASE-LeaseIdCollision {
  lease_a: LeaseId,
  lease_b: LeaseId,
  -- two leases mistakenly assigned the same id
}

LEASE-LeaseKindMismatchAgainstUpstream {
  lease: LeaseId,
  kind: ResourceLeaseKind,
  reason: LeaseKindMismatchReason,
}

pub enum LeaseKindMismatchReason {
  RomWindowBindingNotInRomWindowPlan,
  SramPageBindingNotInSramPagePlan,
  OverlayIdNotInOverlayPlan,
  InterruptMaskPolicyInconsistentWithSlice,
}
```

### 15.3 RES-* codes (residency consistency)

```text
RES-EntryResidencyEpochMismatch {
  slice: SliceId,
  epoch: EpochId,
  slice_entry_residency: Residency,
  epoch_residency: Residency,
}

RES-OverlayInstallEpochMismatch {
  install: InstallId,
  slice: SliceId,
  epoch: EpochId,
  install_region: OverlayId,
  epoch_overlay: Option<OverlayId>,
}

RES-BankSwitchUnbracketed {
  slice: SliceId,
  op_index: usize,
  from: BankId,
  to: BankId,
  reason: BankSwitchUnbracketedReason,
}

pub enum BankSwitchUnbracketedReason {
  FromLeaseNotReleasedBefore,
  ToLeaseNotAcquiredBefore,
  Both,
}

RES-IsrEnabledHoldsRomWindowLease {
  slice: SliceId,
  lease: LeaseId,
  binding: RomWindowBinding,
}

RES-IsrEnabledHoldsSramPageLease {
  slice: SliceId,
  lease: LeaseId,
  binding: SramPageBinding,
}

RES-IsrEnabledInExpertBank {
  slice: SliceId,
  residency: Residency,
}

RES-FaultPathInExpertBank {
  slice: SliceId,
  residency: Residency,
}

RES-ResidencyClassMismatchAgainstRomWindowPlan {
  slice: SliceId,
  declared_class: ResidencyClass,
  rom_window_plan_class: ResidencyClass,
}
```

### 15.4 MODE-* codes (multi-mode pack)

```text
MODE-KeysMismatch {
  modes_keys: Vec<RuntimeMode>,
  epochs_keys: Vec<RuntimeMode>,
  leases_keys: Vec<RuntimeMode>,
}

MODE-CheckpointSchemaMismatch {
  mode: RuntimeMode,
  pack_checkpoint_schema_hash: Hash256,
  mode_checkpoint_schema_hash: Hash256,
}

MODE-ContinuationAbiMismatch {
  mode_a: RuntimeMode,
  mode_b: RuntimeMode,
  hash_a: Hash256,
  hash_b: Hash256,
}

MODE-EntrySliceMissing {
  mode: RuntimeMode,
  declared_entry: SliceId,
}

MODE-PackEmpty {
  -- modes.len() == 0
}

MODE-SwitchPointNotInObservationPlan {
  switch_point: SemanticCheckpointId,
  observation_plan_pin_set_size: u32,
}

MODE-SwitchEpochBoundaryMissing {
  epoch_id: EpochId,
}

MODE-RequestedModeNotEmitted {
  requested: RuntimeMode,
}

MODE-EmittedModeNotRequested {
  emitted: RuntimeMode,
}

MODE-CheckpointPinDivergence {
  mode_a: RuntimeMode,
  mode_b: RuntimeMode,
  pin_set_a: BTreeSet<SemanticCheckpointId>,
  pin_set_b: BTreeSet<SemanticCheckpointId>,
}
```

### 15.5 DRIFT-* codes (drift monitor)

```text
DRIFT-V1MinimumViolated {
  field: DriftEnvelopeField,
  -- e.g. expected.slice_cycles_p95 is None
}

pub enum DriftEnvelopeField {
  ExpectedSliceCyclesP95,
  ExpectedUiCommitCyclesP95,
  ExpectedTraceDropRatePct,
  ExpectedPersistOverrunRatePct,
}

DRIFT-MetricNotInClosedSet {
  metric: u32,
  -- a DriftTrigger.metric outside the closed enum
}

DRIFT-ActionNotInClosedSet {
  action: u32,
  -- a DriftTrigger.action outside the closed enum
}

DRIFT-ObservedNotAllNoneAtCompileTime {
  field: DriftEnvelopeField,
  value: u32,
}

DRIFT-ConsecutiveViolationsNonZeroAtCompileTime {
  value: u8,
}

DRIFT-DemoteModeTargetNotInPack {
  trigger: DriftTrigger,
  target_mode: RuntimeMode,
}
```

### 15.6 LIVENESS-* codes

```text
LIVENESS-NoProgressPerToken {
  total_progress_per_token: u32,  -- == 0
}

LIVENESS-CheckpointPinHasNoSemanticCheckpointOp {
  slice: SliceId,
  pin: SemanticCheckpointId,
}

LIVENESS-MaxNoProgressFramesNotInRuntimeChromeBudget {
  -- The build's RuntimeChromeBudget does not declare max_no_progress_frames.
}
```

### 15.7 Diagnostic ordering

Diagnostics are emitted in an order consistent with the construction
order in §8.3:

1. Self-consistency rules in §8.4 (slice formation), in §-numbered order.
2. Mode-pack invariants in §8.4.7.
3. Mode-switch policy validity in §8.4.8.
4. Drift envelope validity in §8.4.9.

Within each rule, diagnostics for slice/lease/epoch ids are emitted in
ascending id order. This makes the diagnostic stream deterministic for
test assertions.

## 16. Cross-stage interactions

### 16.1 F-B5 (GbInferIR) — semantic equivalence handshake

Stage 10 consumes `GbInferIR` value/effect ids and pins every reachable
`ValueId` and `EffectId` to the schedule:

```text
F-IRRealization:
  ∀ v ∈ GbInferIR.values reachable from any output op.
    ∃ slice s, op_idx o.
      slice s has Load { value: v } or KernelCall producing v
      ∨ s has Store { value: v } realizing v
      ∨ StoragePlan.lookup(v).materialization == Materialize
        and the slice graph contains a Load/Store for v's slot

  ∀ e ∈ GbInferIR.effects reachable from any output op.
    ∃ slice s, op_idx o.
      slice s has Effect { effect: e }
```

If a reachable value or effect is not realized by any slice, Stage 10
emits `SCHED-IrRealizationMissing { value: ValueId } |
SCHED-IrRealizationMissing { effect: EffectId }`. (Added to §15.1's
SCHED-* set.)

This is the **op-for-op correspondence** anchor for F-C3 (`ScheduleOracle`).
F-C3 binds `GbInferIR` ops to slice ops at every checkpoint pin.

### 16.2 F-B6 (ObservationPlan) — checkpoint pinning per slice

Every `SemanticCheckpointId` in `ObservationPlan.semantic_checkpoint_pins`
must appear in exactly one slice's `semantic_checkpoint_pins` (typically
at slice entry or exit). Every `TraceProbeId` must appear in exactly one
slice's `trace_probe_pins`.

```text
F-CheckpointPinning:
  ∀ pin ∈ ObservationPlan.semantic_checkpoint_pins.
    |{ slice s | pin ∈ s.semantic_checkpoint_pins }| == 1
  ∀ probe ∈ ObservationPlan.trace_probe_pins.
    |{ slice s | probe ∈ s.trace_probe_pins }| == 1
```

Diagnostic: `SCHED-CheckpointPinNotRealized { pin } |
SCHED-CheckpointPinDuplicate { pin, slices: Vec<SliceId> }`.

### 16.3 F-B7 (RangePlan) — tile sizes, reduction-loop slice boundaries

Stage 10 consumes `RangePlan` reduction structure and tile sizes to
decide slice boundaries within tile loops. `RangePlan.reduction_sites`
each define a logical reduction whose tile-loop iteration count and
tile size are consumed verbatim. Slice formation cuts the tile loop
into slices whose `hard_cycles_to_safe_point` budget bounds the per-slice
iteration count.

### 16.4 F-B8 (StoragePlan) — Materialization → live_wram/live_sram membership, AliasClassId → lease-correctness assumptions

`StoragePlan.materialization == Materialize { class, lifetime }` for a
value `v` implies `ArenaPlan` allocated a slot for `v`. That slot
appears in `slice.live_wram` (if `class ∈ {WramHot, HramHot}`) or
`slice.live_sram` (if `class == SramPaged`) for every slice that loads
or stores `v`.

`AliasClassId` is preserved verbatim from `StoragePlan` to `ArenaPlan`
to the slice's `live_wram` / `live_sram` slot membership. Two values
with the same `AliasClassId` may share a slot (per F-B12 rules); the
schedule does not re-derive aliasing — it inherits the aliasing
decisions and ensures lease-correctness around them.

### 16.5 F-B9 (SramPagePlan) — SramPageBinding referenced by ResourceLeaseKind::SramPage

Every `ResourceLeaseKind::SramPage(binding)` references an
`SramPageBinding` from `SramPagePlan`. Stage 10 mints exactly one lease
per page-resident binding; Stage 10.5 verifies lease balance per page.

`SramPagePlan.commit_boundaries` are the legal moments at which a
`PersistCommit { group }` SchedOp may appear; Stage 10's slice formation
respects these boundaries.

### 16.6 F-B10 (RomWindowPlan) — RomWindowBinding referenced by ResourceLeaseKind::RomWindow + ResidencyEpoch

Every `ResourceLeaseKind::RomWindow(binding)` references a
`RomWindowBinding` from `RomWindowPlan`. Stage 10 mints exactly one
lease per ROM bank that any slice depends on.

`KernelResidency` from `RomWindowPlan` determines which slice may host
which kernel:

* `KernelResidency::Bank0Fixed` — the kernel is in Bank0; slices that
  call this kernel have `entry_residency = Bank0`.
* `KernelResidency::WramOverlay` — the kernel is in a WRAM overlay;
  slices that call this kernel have `entry_residency = Bank0` but
  reference an `OverlayId` via `OverlayInstall` ops or via held leases.
* `KernelResidency::CoResidentSwitchable` — the kernel and its data
  co-reside in a Common bank; slices that call this kernel have
  `entry_residency = Common(bank_id)`.

`ResidencyEpoch.rom_window` and `ResidencyEpoch.residency` are
constructed to match `KernelResidency`'s implications.

### 16.7 F-B11 (OverlayPlan) — OverlayId referenced by ResourceLeaseKind::Overlay; install events scheduled inside slices

Every `ResourceLeaseKind::Overlay(overlay_id)` references an `OverlayId`
from `OverlayPlan.regions`. `OverlayInstall` SchedOps embedded in slice
op sequences correspond exactly to `OverlayPlan.installs`:

```text
F-OverlayInstallRealization:
  ∀ install ∈ OverlayPlan.installs.
    ∃ slice s, op_idx o.
      s.ops[o] == OverlayInstall { install: install.id }

  ∀ slice s, op_idx o where s.ops[o] == OverlayInstall { install: id }.
    OverlayPlan.installs.contains(id)
    ∧ install.install_event is satisfied by slice s
        (e.g. install.install_event == AtSliceEntry implies
         o == 0 || o == first non-AcquireLease op).
```

Diagnostic: `SCHED-OverlayInstallNotRealized { install }`,
`SCHED-OverlayInstallEventViolated { install, slice, op_index }`.

### 16.8 F-B12 (ArenaPlan) — ArenaSlot referenced by live_wram/live_sram

Every `ArenaSlotRef` in `slice.live_wram` or `slice.live_sram`
references an `ArenaSlot` in `ArenaPlan`. The arena's
`NamedArena` family determines the live-set membership:

* `WramActivationsPing*`, `WramAccumScratch`, `WramRouteScratch`,
  `WramDecodeScratch`, `WramContinuationRecord`, `WramOverlayRegion(_)` →
  `live_wram`.
* `SramSequenceStatePages(_)`, `SramTracePages`,
  `SramHarnessCommandBlock`, `SramHarnessResultBlock`,
  `SramPersistedTranscript`, `SramColdSpill` → `live_sram`.
* `Hram*` → not in slice live sets (HRAM is implicitly always live;
  shadow registers and frame flags are not slice-scoped).

```text
F-LiveSetMembershipMatchesArenaFamily:
  ∀ s ∈ slices.
    ∀ r ∈ s.live_wram. ArenaPlan.lookup(r).named ∈ WramFamily
    ∀ r ∈ s.live_sram. ArenaPlan.lookup(r).named ∈ SramFamily
```

Diagnostic: `SCHED-LiveSetFamilyMismatch { slice, slot, expected_family, actual_family }`.

### 16.9 F-B14 (ScheduleCostAnalysis) — consumes SchedulePack to produce per-mode cost envelopes

F-B14 consumes `SchedulePack` and produces per-mode cost envelopes by:

* charging actual cycles (from calibration) against
  `SchedSlice.soft_target_cycles`;
* checking actual cycles against `SchedSlice.hard_cycles_to_safe_point`
  (a hard ceiling — exceeding it is a build-rejection);
* aggregating per-mode `ResourceVector` totals and comparing against
  `RuntimeChromeBudget` per-token caps;
* emitting `schedule_cost.json` per the F-B14 RFC.

F-B13 does not consume calibration; F-B14 does. The seam is clean: F-B13
declares budgets, F-B14 charges actuals.

### 16.10 F-B15 (Backend) — consumes SchedulePack; ReachabilityValidation is the computed version of ISR/yield-residency rules

F-B15's pipeline:

1. `AsmIR` lowering — for each slice, lower SchedOps to AsmIR ops. The
   slice's contract (entry_residency, interrupt_policy, required_leases,
   yield_kind, exit_kind) is honored verbatim.
2. `ReachabilityValidation` — whole-program reachability after far-call
   legalization. The computed counterpart to F-B13's annotation-driven
   §9.1.3 check. Updates
   `resource_state.cert.json.isr_visible_residency.computed_reachability_confirmed`
   to `true`, or emits `certs/reachability.cert.json` superseding the
   annotation evidence.
3. `PlacedRom` — section ordering, far-call thunks, branch relaxation.
   Section addresses respect `ArenaPlan` byte ranges (for arena-resident
   sections) and `RomWindowPlan` bank assignments.
4. `EncodedRom` — final byte emission.

### 16.11 F-B16 (FeasibilityRefinementLoop, BLOCKED on oracle) — RepairProposals against CompileKnobs

F-B16 wraps stages 5–11 (including F-B13). Repair proposals against
`CompileKnobs` may shrink slices, demote trace, or promote overlay,
producing a new `ResolvedCompilePolicy` and triggering a re-run of
F-B13.

F-B13's data layout leaves `RepairProposal` pluggable:

* every `Diagnostic.provenance` may contain a `RepairProposal` reference
  in v2 (currently `PolicySource ⊆ {TargetDefault, ProfileDefault,
  CompileRequestOverride, HintBundle, Calibration}`);
* every `CompileKnobsSection` provenance entry may reference a
  `RepairProposalId` (v2 only);
* the `Diagnostic.detail` field may carry a `RepairHintTaxonomy`
  (v2 only).

These are forward-compatible schema features; v1 emits no
RepairProposal-derived data.

### 16.12 F-B17 (StageCache integration sweep)

F-B17 unifies the cache key construction across all stages. F-B13 uses
the existing F-B2/F-B4 convention until F-B17 lands. F-B17 may amend
this RFC's `feature_set_hash` field to use the shared canonical
representation.

### 16.13 Runtime: F-D1 (cooperative-kernel scheduler)

F-D1 is the runtime side of the SchedulePack contract. The runtime:

* hosts one `RuntimeMode`'s `GbSchedIR` at any moment;
* observes per-slice cycles and updates `RuntimeDriftMonitor.observed`;
* dispatches `DriftAction`s when triggers fire;
* honors `ModeSwitchPolicy.legal_switch_points` for runtime-initiated
  mode switches;
* preserves `gbf-abi::InferenceState` continuity across slice boundaries.

The runtime does **not** mutate `SchedulePack` at runtime. Mode
switches are *selections* among the modes the build emitted.

### 16.14 Runtime: F-D6 (SchedulePack mode switching, BLOCKED on oracle)

F-D6 implements the runtime-side mode-switch executor: state save in
mode A, mode swap, state restore in mode B. F-D6 consumes `SchedulePack`
verbatim; F-B13 owns the schema; F-D6 owns the execution logic.

### 16.15 Sister: F-C3 (ScheduleOracle)

F-C3 is the schedule oracle that binds emulator harness state to slice
boundaries. Its op-for-op correspondence target is the slice's op
sequence. F-C3 consumes `SchedulePack` verbatim and tracks
slice-boundary state for emulator-vs-oracle diffing.

### 16.16 Sister: F-F2 (Certificates)

F-F2 consumes `certs/resource_state.cert.json` as part of the certified
build artifact set. F-F2 also consumes `certs/range.cert.json`,
`certs/arena.cert.json`, `certs/window.cert.json`, `certs/sram.cert.json`,
and (post-F-B15) `certs/reachability.cert.json`.

## 17. Task DAG

This RFC mints task beads under bd-9ae. The compressed DAG:

```text
T-B13.1   Wave-0 type stubs in gbf-codegen and gbf-policy
            -- introduces SchedSlice, SchedOp, ResourceLease,
               ResourceLeaseKind, ResourceVector, ResidencyEpoch,
               SchedulePack, ModeSwitchPolicy, RuntimeMode,
               RuntimeDriftMonitor, DriftEnvelope, DriftTrigger,
               DriftAction, DriftMetric, YieldKind, YieldCheckClass,
               ExitKind, InterruptPolicy, LeaseId, EpochId, SliceId,
               CycleBudget, ArenaSlotRef, ScratchSlot
            -- schema-only; no validator dispatch
            -- depends on: F-B2 §2.14 absorption (closed), F-B11/F-B12
               types (consumed by hash)

T-B13.2   Stage 10 input plumbing
            -- introduces SchedIrInputs, SchedIrInputsBuilder
            -- gathers references to all upstream products
            -- depends on: T-B13.1; upstream stage products (F-B5..F-B12)

T-B13.3   Residency-epoch construction (§8.3 step 1)
            -- pure function: RomWindowPlan + OverlayPlan ->
                 Vec<ResidencyEpoch>
            -- depends on: T-B13.1, T-B13.2

T-B13.4   Slice formation (§8.3 step 2)
            -- pure function: GbInferIR + RangePlan + StoragePlan +
                 ArenaPlan + ResolvedCompilePolicy ->
                 Vec<SchedSlice>
            -- depends on: T-B13.3

T-B13.5   Lease binding (§8.3 step 3)
            -- pure function: SchedSlice + RomWindowPlan + SramPagePlan
                 + OverlayPlan -> Vec<ResourceLease>
            -- depends on: T-B13.4

T-B13.6   Acquire/Release op insertion (§8.3 step 4)
            -- modifies slice.ops in place to insert AcquireLease /
               ReleaseLease ops
            -- depends on: T-B13.5

T-B13.7   Checkpoint pinning (§8.3 step 5)
            -- pure function: ObservationPlan + Vec<SchedSlice> ->
                 augmented Vec<SchedSlice> with semantic_checkpoint_pins
                 and trace_probe_pins set
            -- depends on: T-B13.6

T-B13.8   Mode-pack assembly (§8.3 step 6)
            -- pure function: per-mode SchedSlice/Lease/Epoch ->
                 SchedulePack
            -- depends on: T-B13.7

T-B13.9   ModeSwitchPolicy assembly (§8.3 step 7)
            -- pure function: ResolvedCompilePolicy + per-mode epochs
                 + ObservationPlan -> ModeSwitchPolicy
            -- depends on: T-B13.8

T-B13.10  Drift envelope binding (§8.3 step 8)
            -- pure function: ResolvedCompilePolicy + slice budgets ->
                 RuntimeDriftMonitor
            -- depends on: T-B13.9

T-B13.11  Stage 10 self-consistency rules (§8.4)
            -- emits hard diagnostics on rule violation
            -- depends on: T-B13.10

T-B13.12  Stage 10 driver and StageCache wiring (K10)
            -- emits sched_ir.json, slice_report.json
            -- depends on: T-B13.11; F-B17 (deferred for v1)

T-B13.13  Lease-flow analysis (§9.3.1)
            -- pure function: SchedulePack -> Vec<LeaseBalanceFact> +
                 diagnostics
            -- depends on: T-B13.12

T-B13.14  Yield-safety analysis (§9.3.2)
            -- pure function: SchedulePack -> Vec<YieldSafetyFact> +
                 diagnostics
            -- depends on: T-B13.13

T-B13.15  ISR-visible residency analysis (§9.3.3)
            -- pure function: SchedulePack -> Vec<IsrVisibleResidencyFact>
                 + diagnostics
            -- annotation-driven; computed-reachability_confirmed = false
            -- depends on: T-B13.14

T-B13.16  Overlay/bank-shadow consistency analysis (§9.3.4)
            -- pure function: SchedulePack ->
                 Vec<OverlayBankShadowConsistencyFact> + diagnostics
            -- depends on: T-B13.15

T-B13.17  Liveness analysis (§12)
            -- pure function: SchedulePack -> LivenessSection + diagnostics
            -- depends on: T-B13.16

T-B13.18  Stage 10.5 driver and StageCache wiring (K10.5)
            -- emits certs/resource_state.cert.json
            -- depends on: T-B13.13..T-B13.17

T-B13.19  Schema/round-trip tests for sched_ir.v1, slice_report.v1,
          resource_state.cert.v1
            -- includes self-hash round trip, deny_unknown_fields,
               canonical-json determinism
            -- depends on: T-B13.12, T-B13.18

T-B13.20  Synthetic fixture builds for v1 closure
            -- includes:
                 (a) single-mode build (Default profile, one mode);
                 (b) multi-mode build (Bringup profile, three modes,
                     no legal switch points);
                 (c) multi-mode build with one legal switch point;
                 (d) intentional-violation builds for every diagnostic
                     code in §15 (one fixture per code; verifies
                     diagnostic emission)
            -- depends on: T-B13.19

T-B13.21  Independent verifier in gbf-verify
            -- re-runs §9.1 proof obligations against the certificate
            -- depends on: T-B13.20

T-B13.22  Mint follow-up beads
            -- F-B14 wiring (consumes SchedulePack)
            -- F-B15 wiring (consumes SchedulePack)
            -- F-D1 wiring (cooperative-kernel scheduler consumes
               SchedulePack)
            -- F-C3 wiring (ScheduleOracle consumes SchedulePack)
            -- F-D6 prep (BLOCKED on oracle; wires schema only)
```

**Parallelization opportunity:** T-B13.13, T-B13.14, T-B13.15, T-B13.16,
T-B13.17 are all independent functions of `SchedulePack`. They may be
implemented in parallel and tested independently before T-B13.18 stitches
them together.

**Critical path:** T-B13.1 → T-B13.4 → T-B13.6 → T-B13.8 → T-B13.11 →
T-B13.12 → (parallel batch) → T-B13.18 → T-B13.20.

## 18. Rejection classes (closure gate)

Closure of bd-9ae requires every rejection class below to have at least
one synthetic-fixture test that verifies the diagnostic is emitted.

### 18.1 Stage 10 self-consistency rejections

| Class                                   | Diagnostic code                                         |
|-----------------------------------------|---------------------------------------------------------|
| Required-lease not in scope             | `LEASE-RequiredLeaseNotAcquired`                        |
| Live slot lifetime mismatch             | `SCHED-LiveSlotLifetimeMismatch`                        |
| Hard cycles below interrupt latency     | `SCHED-HardLatencyBelowInterruptLatency`                |
| Soft target above hard bound            | `SCHED-SoftTargetExceedsHardBound`                      |
| Epoch coverage gap                      | `SCHED-EpochCoverageGap`                                |
| Epoch overlap                           | `SCHED-EpochOverlap`                                    |
| Storage class drifted                   | `SCHED-StorageClassDrifted`                             |
| Op sequence malformed                   | `SCHED-OpSequenceMalformed`                             |
| Terminal op inconsistent with exit kind | `SCHED-TerminalOpInconsistentWithExitKind`              |
| Load/Store slot not in live set         | `SCHED-LoadStoreSlotNotInLiveSet`                       |
| Kernel call residency mismatch          | `SCHED-KernelCallResidencyMismatch`                     |
| Overlay install lease unsatisfied       | `SCHED-OverlayInstallLeaseShapeUnsatisfied`             |
| Tail call entry residency mismatch      | `SCHED-TailCallEntryResidencyMismatch`                  |
| Tail call lease set mismatch            | `SCHED-TailCallLeaseSetMismatch`                        |
| EnterIsr in non-ISR slice               | `SCHED-EnterIsrInNonIsrSlice`                           |
| ISR entry slice not Bank0               | `SCHED-IsrEntrySliceNotBank0`                           |
| Fault path residency mismatch           | `SCHED-FaultPathResidencyMismatch`                      |
| Cycle budget overflow                   | `SCHED-CycleBudgetOverflow`                             |
| Yield safety table violation            | `LEASE-YieldSafetyTableViolation`                       |
| Lease kind mismatch against upstream    | `LEASE-LeaseKindMismatchAgainstUpstream`                |
| Acquire/release scope violation         | `LEASE-AcquireReleaseScopeViolation`                    |
| Lease id collision                      | `LEASE-LeaseIdCollision`                                |
| IR realization missing                  | `SCHED-IrRealizationMissing`                            |
| Checkpoint pin not realized             | `SCHED-CheckpointPinNotRealized`                        |
| Checkpoint pin duplicate                | `SCHED-CheckpointPinDuplicate`                          |
| Overlay install not realized            | `SCHED-OverlayInstallNotRealized`                       |
| Overlay install event violated          | `SCHED-OverlayInstallEventViolated`                     |
| Live set family mismatch                | `SCHED-LiveSetFamilyMismatch`                           |
| Residency class mismatch                | `RES-ResidencyClassMismatchAgainstRomWindowPlan`        |

### 18.2 Mode-pack rejections

| Class                                   | Diagnostic code                                         |
|-----------------------------------------|---------------------------------------------------------|
| Pack keys mismatch                      | `MODE-KeysMismatch`                                     |
| Checkpoint schema mismatch              | `MODE-CheckpointSchemaMismatch`                         |
| Continuation ABI mismatch               | `MODE-ContinuationAbiMismatch`                          |
| Entry slice missing                     | `MODE-EntrySliceMissing`                                |
| Pack empty                              | `MODE-PackEmpty`                                        |
| Switch point not in observation plan    | `MODE-SwitchPointNotInObservationPlan`                  |
| Switch epoch boundary missing           | `MODE-SwitchEpochBoundaryMissing`                       |
| Requested mode not emitted              | `MODE-RequestedModeNotEmitted`                          |
| Emitted mode not requested              | `MODE-EmittedModeNotRequested`                          |
| Checkpoint pin divergence               | `MODE-CheckpointPinDivergence`                          |

### 18.3 Drift rejections

| Class                                   | Diagnostic code                                         |
|-----------------------------------------|---------------------------------------------------------|
| V1 minimum violated (slice_cycles_p95)  | `DRIFT-V1MinimumViolated`                               |
| Drift metric not in closed set          | `DRIFT-MetricNotInClosedSet`                            |
| Drift action not in closed set          | `DRIFT-ActionNotInClosedSet`                            |
| Observed not all-None at compile time   | `DRIFT-ObservedNotAllNoneAtCompileTime`                 |
| Consecutive violations non-zero compile | `DRIFT-ConsecutiveViolationsNonZeroAtCompileTime`       |
| DemoteMode target not in pack           | `DRIFT-DemoteModeTargetNotInPack`                       |

### 18.4 Stage 10.5 (resource_state) rejections

| Class                                   | Diagnostic code                                         |
|-----------------------------------------|---------------------------------------------------------|
| Lease unbalanced                        | `LEASE-Unbalanced`                                      |
| Lease double-acquired                   | `LEASE-DoubleAcquire`                                   |
| Release without acquire                 | `LEASE-ReleaseWithoutAcquire`                           |
| Path-divergent lease state              | `LEASE-PathDivergent`                                   |
| Yield crosses non-resumable lease       | `LEASE-YieldCrossesNonResumable`                        |
| Entry residency epoch mismatch          | `RES-EntryResidencyEpochMismatch`                       |
| Overlay install epoch mismatch          | `RES-OverlayInstallEpochMismatch`                       |
| Bank switch unbracketed                 | `RES-BankSwitchUnbracketed`                             |
| ISR-enabled holds RomWindow lease       | `RES-IsrEnabledHoldsRomWindowLease`                     |
| ISR-enabled holds SramPage lease        | `RES-IsrEnabledHoldsSramPageLease`                      |
| ISR-enabled in expert bank              | `RES-IsrEnabledInExpertBank`                            |
| Fault path in expert bank               | `RES-FaultPathInExpertBank`                             |

### 18.5 Liveness rejections

| Class                                   | Diagnostic code                                         |
|-----------------------------------------|---------------------------------------------------------|
| No progress per token                   | `LIVENESS-NoProgressPerToken`                           |
| Checkpoint pin missing op               | `LIVENESS-CheckpointPinHasNoSemanticCheckpointOp`       |
| max_no_progress_frames missing          | `LIVENESS-MaxNoProgressFramesNotInRuntimeChromeBudget`  |

### 18.6 Closure gate

For chunk closure (bd-9ae):

* every rejection class above has at least one synthetic-fixture test
  that triggers the diagnostic;
* every passing fixture round-trips through the schema validator and
  self-hash;
* the deterministic regenerator emits byte-identical outputs across two
  consecutive runs;
* `gbf-verify`'s independent re-checker accepts every passing
  certificate;
* `cargo test --workspace --all-features -- F-B13` passes;
* the review packet under `docs/review/f-b13/` is current.

## 19. Proof obligations (formal closure gates)

These are the formal claims the chunk closure stands on. Each is a
mathematically-shaped statement; the implementation discharges each via
the cited algorithm in §9.3 or via Stage 10 self-consistency in §8.4.

### 19.1 Determinism

```text
F-Det-Stage10 (T-B13.12 verifies):
  Same SchedIrInputs ⇒ byte-identical SchedulePack and sched_ir.json.

F-Det-Stage10_5 (T-B13.18 verifies):
  Same SchedulePack ⇒ byte-identical certs/resource_state.cert.json.

F-Det-Pack-CanonicalSort (T-B13.11 verifies):
  Slices in SchedulePack[mode].slices are sorted by SliceId ascending.
  Epochs in SchedulePack[mode].epochs are sorted by EpochId ascending.
  Leases in SchedulePack[mode].leases are sorted by LeaseId ascending.
  Modes in SchedulePack.modes (and .epochs, .leases) iterate in
    BTreeMap key order (RuntimeMode discriminant ascending).
```

### 19.2 Lease balance

```text
F-LeaseBalance (T-B13.13 verifies):
  ∀ l ∈ SchedulePack[mode].leases.
    On every path from entry slice to any terminal slice
    (Halt, Fault, or post-Yield slice's run-to-end behavior),
    acquire_count(l, path) == release_count(l, path)
    and at every prefix, acquire_count >= release_count.
```

### 19.3 Yield safety

```text
F-YieldSafety (T-B13.14 verifies):
  ∀ s ∈ SchedulePack[mode].slices with exit_kind = SaveContinuationAndYield.
    let outstanding_at_exit = leases held just before terminal Yield op
    ∀ l ∈ outstanding_at_exit. l.yield_safe == true.
```

### 19.4 ISR-visible residency (annotation-driven)

```text
F-IsrVisibleResidency (T-B13.15 verifies):
  ∀ s ∈ SchedulePack[mode].slices with interrupt_policy = Enabled.
    s.entry_residency ∈ {Bank0, Common(_)}
    ∧ at every op in s, no outstanding lease has kind ∈
        {RomWindow(_), SramPage(_)}.

  Note: F-B15 ReachabilityValidation supersedes this with the computed
  whole-program counterpart; the certificate field
  `computed_reachability_confirmed` is false in v1.
```

### 19.5 Overlay/bank-shadow consistency

```text
F-OverlayBankShadowConsistency (T-B13.16 verifies):
  ∀ s ∈ SchedulePack[mode].slices.
    ∃! e ∈ SchedulePack[mode].epochs with s.id ∈ e.slices.
    s.entry_residency == e.residency
    ∧ ∀ OverlayInstall(install) op in s.ops:
        OverlayPlan.lookup(install).region == e.overlay
    ∧ ∀ BankSwitch(from, to) op at index o in s.ops:
        ∃ ReleaseLease(l1) op at index < o in s.ops with
          l1.kind == RomWindow(_) ∧ l1.bank_id == from
        ∧ ∃ AcquireLease(l2) op at index < o in s.ops with
          l2.kind == RomWindow(_) ∧ l2.bank_id == to.
```

### 19.6 Mode-pack equivalence

```text
F-ModePackEquivalence (T-B13.11 verifies):
  ∀ mode_a, mode_b ∈ SchedulePack.modes.keys().
    Hash256(SchedulePack.modes[mode_a].checkpoint_schema) ==
    Hash256(SchedulePack.modes[mode_b].checkpoint_schema) ==
    SchedulePack.checkpoint_schema_hash

    ∧ Hash256(continuation_abi(SchedulePack.modes[mode_a])) ==
      Hash256(continuation_abi(SchedulePack.modes[mode_b])) ==
      SchedulePack.continuation_abi_hash

    ∧ ObservationPlan-pinned SemanticCheckpointId set is realized
      identically in every mode (every pin appears once in some slice
      of every mode).

    ∧ The set of EffectId edges traversed in topological order is
      identical across modes (same effect linearization).
```

### 19.7 Liveness

```text
F-Liveness (T-B13.17 verifies):
  Per-token total progress_epoch increment count > 0.
  Equivalently: every output token has at least one SemanticCheckpoint
  op on its compute path that increments progress_epoch.
```

### 19.8 Realization (semantic equivalence with prior IR)

```text
F-IRRealization (T-B13.4 / T-B13.7 verify):
  ∀ v ∈ GbInferIR.values reachable from any output op.
    ∃ slice s, op_idx o.
      slice s realizes v through Load, Store, KernelCall, or
      Materialize-derived live_wram/live_sram membership.

  ∀ e ∈ GbInferIR.effects reachable from any output op.
    ∃ slice s, op_idx o.
      slice s has Effect { effect: e } at op_idx o.
```

### 19.9 Cache discipline

```text
F-Cache-Stage10 (T-B13.12 verifies):
  Two builds with identical K10 inputs hit the cache; one byte changed
  in any K10 input misses.

F-Cache-Stage10_5 (T-B13.18 verifies):
  Two builds with identical K10.5 inputs (sched_ir self_hash) hit the
  cache; one byte changed in the SchedulePack misses.

F-Cache-Independence (T-B13.18 verifies):
  A K10 cache hit does NOT skip Stage 10.5. K10.5 is checked
  independently.
```

### 19.10 Schema stability

```text
F-Schema-V1 (T-B13.19 verifies):
  sched_ir.v1, slice_report.v1, resource_state.cert.v1 schemas are
  stable: every public field is documented; every closed enum has
  exhaustive variants; round-trip serde with deny_unknown_fields
  rejects unknown fields and unknown enum variants.

F-NoFloat (T-B13.19 verifies):
  No floating-point JSON numbers in any v1 report or certificate.
```

## 20. End-to-end theorem

**Theorem (Stage 10 + Stage 10.5 closure).** When Stage 10 emits a
`SchedulePack` and Stage 10.5 emits a `ResourceStateCertificate` with
`outcome = Passed`, then for every `RuntimeMode mode` in
`SchedulePack.modes.keys()`:

```text
1. Interrupt safety:
   Every slice s in SchedulePack[mode].slices satisfies
     s.hard_cycles_to_safe_point >= s.max_interrupt_latency,
     s.soft_target_cycles <= s.hard_cycles_to_safe_point,
   and the slice's interrupt_policy is consistent with the leases
   it holds (Enabled ⇒ no RomWindow/SramPage lease held during any op
   of s; ShortCriticalSection / Disabled may hold an InterruptMask
   lease).

2. Lease balance:
   Every lease l in SchedulePack[mode].leases is acquired and released
   exactly once on every reachable slice path.

3. Yield safety:
   Every slice s with exit_kind = SaveContinuationAndYield has
   outstanding lease set at exit such that every member has
   yield_safe = true.

4. Residency correctness against the chosen RuntimeMode:
   Every slice s belongs to exactly one ResidencyEpoch e with
   s.entry_residency == e.residency. Every BankSwitch op is
   bracketed by matching RomWindow lease release/acquire events.
   Every OverlayInstall op aligns with the epoch's overlay set.

5. ISR-visible residency (annotation-driven):
   Every slice s with interrupt_policy = Enabled has
   s.entry_residency ∈ {Bank0, Common(_)} and holds no RomWindow or
   SramPage lease at any of its ops.
   (F-B15 supersedes with computed whole-program reachability.)

6. Liveness:
   Total per-token progress_epoch increments > 0; every output token
   has at least one SemanticCheckpoint pin on its compute path.

7. Mode equivalence:
   Every mode in SchedulePack shares the same checkpoint_schema_hash,
   the same continuation_abi_hash, and the same set of
   ObservationPlan-pinned SemanticCheckpointIds; modes differ only in
   tile sizes, slice composition, yield spacing, and trace density.

8. Drift envelope contract:
   SchedulePack.drift_monitor.expected.slice_cycles_p95.is_some();
   every DriftTrigger.metric is in the closed DriftMetric set; every
   DriftTrigger.action is in the closed DriftAction set; observed is
   all-None at compile time; consecutive_violations is 0 at compile
   time.

9. Determinism:
   Two builds with identical SchedIrInputs produce byte-identical
   SchedulePacks and sched_ir.json. Two builds with identical
   SchedulePacks produce byte-identical certs/resource_state.cert.json.

10. Cache discipline:
    K10 and K10.5 are content-addressed against canonical-JSON
    representations of their typed inputs. Stage 10.5 is independently
    cacheable; a Stage 10 cache hit does not bypass Stage 10.5.
```

**Proof sketch.** Each numbered claim is the conjunction of:

* a Stage 10 self-consistency rule (§8.4) discharged before
  `SchedulePack` is emitted;
* a Stage 10.5 lease-flow analysis fact (§9.3) discharged before
  `certs/resource_state.cert.json` is emitted with `outcome = Passed`.

Specifically:

* **(1)** Interrupt safety: §8.4.4 (hard ≥ max, soft ≤ hard); §9.3.3
  (ISR-enabled lease residency); §8.4.5 (yield-safety table).
* **(2)** Lease balance: §9.3.1's flow-graph traversal proves the
  formal predicate F-LeaseBalance.
* **(3)** Yield safety: §9.3.2's per-yield-event analysis proves
  F-YieldSafety.
* **(4)** Residency correctness: §9.3.4's per-slice/per-op analysis
  proves F-OverlayBankShadowConsistency.
* **(5)** ISR-visible residency (annotation-driven): §9.3.3 proves
  F-IsrVisibleResidency. F-B15's ReachabilityValidation supersedes
  with computed reachability.
* **(6)** Liveness: §12 and T-B13.17's per-slice analysis proves
  F-Liveness.
* **(7)** Mode equivalence: §8.4.7 proves F-ModePackEquivalence.
* **(8)** Drift envelope contract: §8.4.9 proves F-DriftEnvelopeV1Minimum
  and the closed-set membership.
* **(9)** Determinism: T-B13.12 / T-B13.18's regression tests verify
  byte-identical regeneration.
* **(10)** Cache discipline: T-B13.12 / T-B13.18's regression tests
  verify cache-hit vs cache-miss behavior.

**Caveat (annotation vs computed).** Claim (5) is annotation-driven in
v1. The build is *locally honest* — every slice's declared residency is
consistent with the upstream plans. F-B15's `ReachabilityValidation`
is the *globally honest* counterpart. Builds that pass Stage 10.5 but
fail F-B15 Reachability are signaled to F-B16 (BLOCKED) for repair-loop
iteration; this RFC does not consume repair proposals.

**Caveat (cycle costs).** Claims (1) hard-cycle bound and `soft_target`
are upper bounds and predictions respectively at the schedule level;
F-B14 charges actual cycles against calibration. A SchedulePack that
passes Stage 10.5 may still fail F-B14 if calibration reveals a slice's
actual cycles exceed `hard_cycles_to_safe_point`. Such failures
trigger F-B16 (BLOCKED) repair-loop iteration.

**Caveat (modes share semantics, not bytes).** Claim (7) is at the
*semantics* level. Different modes may emit different AsmIR (different
tile sizes, different yield-check classes, different overlay install
patterns). F-B15's backend honors per-mode AsmIR; the artifact carries
all modes as a multi-section artifact. Mode switching at runtime
re-binds the active section set.

## 21. Final concise contract

**F-B13 owns the schedule + proof pair.** Two stages, one feature, one
chunk:

* **Stage 10 GbSchedIR** commits the value/effect IR + storage/range/
  SRAM/ROM/overlay/arena plans into slices, leases, residency epochs,
  a multi-mode `SchedulePack`, a `ModeSwitchPolicy`, and a
  `RuntimeDriftMonitor`. Slices have bounded interrupt latency,
  declared interrupt policy, declared entry residency, declared lease
  set, and a yielding-or-fault-class exit. Multi-mode is keyed by
  `RuntimeMode`; modes share artifact semantics, checkpoint schema,
  and continuation ABI.

* **Stage 10.5 ResourceStateValidation** runs typed lease-flow analysis
  over the slice graph and proves four properties: lease balance,
  yield safety, ISR-visible residency (annotation-driven; F-B15
  supersedes with computed), and overlay/bank-shadow consistency.
  Emits `certs/resource_state.cert.json`.

**Inputs (by hash):** `GbInferIR`, `ObservationPlan`, `RangePlan`,
`StoragePlan`, `SramPagePlan`, `RomWindowPlan`, `OverlayPlan`,
`ArenaPlan`, `ResolvedCompilePolicy`, `RuntimeChromeBudget`.

**Outputs:**
* `sched_ir.json` — Stage 10 product report.
* `slice_report.json` — per-slice histograms.
* `certs/resource_state.cert.json` — Stage 10.5 typed certificate.
* `SchedulePack` (in-memory product) — consumed by F-B14, F-B15,
  F-C3, F-D1.

**StageCache keys:** K10 (Stage 10), K10.5 (Stage 10.5).

**v1 closure surface:**
* one default mode (Default profile) or three modes (Bringup profile);
* `expected.slice_cycles_p95.is_some()`;
* `observed = DriftEnvelope::all_none()`, `consecutive_violations = 0`;
* `legal_switch_points` may be empty for v1 (multi-mode is schema-only
  until F-D6 unblocks);
* `computed_reachability_confirmed = false` (F-B15 supersedes).

**Inheritance:** F-B2/F-B4 (envelope, JSON, hash, StageCache,
diagnostic taxonomy); F-B3/F-B5 (`GbInferIR`, no-partial-product law);
F-B11/F-B12 (`OverlayId`, `ArenaSlot`, `NamedArena`, `OverlayInstall`);
F-B6 (`SemanticCheckpointId`, `TraceProbeId`); F-B7 (`RangePlan`);
F-B8 (`Materialization`, `LifetimeClass`, `AliasClassId`); F-B9
(`SramPageBinding`); F-B10 (`RomWindowBinding`, `KernelResidency`,
`Residency`); F-A4 (`BankLease`/`BankGuard` ABI mirror); F-A5 (Bank0
runtime conventions).

**Forward-compatible amendments:** the schema reserves room for
`RepairProposal` source values in `Diagnostic.provenance`,
`RepairProposalId` references in `CompileKnobsSection.provenance`, and
`computed_reachability_confirmed = true` confirmations from F-B15.
F-B16 (BLOCKED) and F-B15 (next chunk) own those amendments.

**The big picture in one sentence.** F-B13 turns "what the program
means" into "how the cooperative kernel runs it on real Game Boy
hardware with bounded interrupt latency, balanced leases, residency-
honest slices, and observable runtime drift" — and emits a typed
certificate that proves it.

End of RFC F-B13.
