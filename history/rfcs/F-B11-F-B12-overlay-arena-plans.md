# RFC F-B11 + F-B12: Spatial Plans — `OverlayPlan` (Stage 8.5) and `ArenaPlan` (Stage 9)

## -1. Authority and amendment policy

This RFC is the source of truth for F-B11 and F-B12 implementation.
`history/planv0.md` remains the architectural context document, but this RFC
is allowed to refine, narrow, or supersede `planv0.md` wherever this RFC makes
a more precise implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence
must be recorded in an `Amends planv0` note close to the relevant decision.
This is not a request to edit `planv0.md` immediately; it is a local
source-of-truth ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B11/F-B12 behavior, this RFC
  wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, StageCache key construction, `ReportEnvelope` shape), the
  F-B2/F-B4 RFC wins. F-B11/F-B12 inherit those surfaces unchanged unless
  this RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on `QuantGraph` or
  `GbInferIR` shape or canonical-product handling, the F-B3/F-B5 RFC wins.
* F-B8 (`StoragePlan`), F-B9 (`SramPagePlan`), and F-B10 (`RomWindowPlan`)
  RFCs are forthcoming. This RFC consumes their public types and
  reportable identities by hash; if a forthcoming RFC changes those public
  types, that RFC must explicitly amend this RFC.
* If a later RFC changes any public type, report shape, cache key,
  diagnostic code, or canonicalization rule introduced here, that later
  RFC must explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature beads   | bd-140 **F-B11 OverlayPlan (Stage 8.5)**; bd-3bw **F-B12 ArenaPlan (Stage 9)** |
| Open tasks      | To be minted: T-B11.1..T-B11.N (overlay region binding, install scheduling, share-class assembly, residency-input handshake, reservation accounting, `overlay_plan.json` emitter, optional `certs/overlay.cert.json`, schema/round-trip tests, StageCache wiring); T-B12.1..T-B12.M (`NamedArena` enum + arena registry, `ArenaSlot` allocation, alias-class merge, lifetime preservation, persistent-page geometry, reservation honoring, `arena_plan.json` emitter, `certs/arena.cert.json` emitter, schema/round-trip tests, StageCache wiring) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 113–212 (target, region map, regions sizes); 1665–1745 (Stage 6 `StoragePlan`); 1727–1755 (Stage 8 `RomWindowPlan`); 1743–1770 (Stage 8.5 `OverlayPlan` + Stage 9 `ArenaPlan`); 1770–1900 (Stage 10 `GbSchedIR`); 1989–2210 (runtime architecture, memory plan, persistent record protocol); 2640–2870 (tests, certificates, `arena.cert.json`) |
| Glossary        | `history/glossary.md` (residency, common bank, expert bank, arena, persistent record, BankLease, Bank0, WRAM overlay, page state, commit group) |
| Constitution    | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation (provides `ReportEnvelope`, `ValidationDiagnostic`, canonical JSON / self-hash, StageCache key construction); F-B3/F-B5 Canonical IRs (provides `QuantGraph`, `GbInferIR` consumed transitively through later stage products); F-B8 StoragePlan (forthcoming) — `StorageBinding`, `LifetimeClass`, `Materialization`; F-B9 SramPagePlan (forthcoming) — `SramPagePlan` reservation handshake; F-B10 RomWindowPlan (forthcoming) — `KernelResidency::WramOverlay` + `RomWindowPlan` identity; F-B13 GbSchedIR + ResourceStateValidation (consumes both products); F-B17 StageCache integration sweep; F-A4 BankLease/BankGuard ABI (overlay installs and arena harness blocks must respect lease ABI); F-A5 Bank0 runtime (Bank0 budget for overlay-install trampolines and arena harness command/result blocks) |
| Sister deps     | F-B16 FeasibilityRefinementLoop (BLOCKED on oracle question) — both products feed it; F-C3 ScheduleOracle (consumes ArenaPlan downstream) |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B11 + F-B12 sits inside the
compiler-pipeline epic, where that epic sits inside the full project, and
which adjacent chunks' contracts this RFC inherits or honors.

### 0.1 Project at a glance — the eight epics

The gbllm project compiles a tiny language model into an LR35902 ROM that
runs on real Game Boy hardware. The work is split across eight epics
(`planv0.md` §"Workspace skeleton"):

```text
Epic A — M0 Foundation Stack
          gbf-asm, gbf-hw, gbf-abi, BankLease/BankGuard, Bank0 runtime,
          gbf-emu, gbf-debug, gbf-store. Provides the target/abi/asm
          contracts every other epic builds on.

Epic B — Compiler Pipeline (14 stages + refinement loop)        ← THIS EPIC
          The transform pipeline from frozen ArtifactCore +
          CompileRequest to a CompiledBuild (ROM + reports + certificates).

Epic C — Oracle Stack
          DenotationalOracle (F-C1), ArtifactOracle (F-C2),
          ScheduleOracle (F-C3), ConformanceEnvelope (F-C4).

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack.

Epic E — Calibration & Bench
          gbf-bench: cycle calibration, kernel timing, autotune.

Epic F — Reports & Verify
          gbf-report (build reports, certificates) + gbf-verify
          (independent slow reference implementations).

Epic G — Data, Lexical, Decode Pipeline
          gbf-data (corpus, charset, normalization, decode policy).

Epic H — Kernel
          gbf-kernel (KernelSpec + matvec/residual/norm/route/decode kernel
          implementations).
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
  F-B11 Stage 8.5      OverlayPlan                                   ← THIS RFC
  F-B12 Stage 9        ArenaPlan                                     ← THIS RFC
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
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
Chunk 3 (next up):    F-B6 + F-B7         Stages 4, 5
Chunk 4:              F-B8                Stage 6
Chunk 5:              F-B9 + F-B10        Stages 7, 8
Chunk 6 (THIS RFC):   F-B11 + F-B12       Stages 8.5, 9
Chunk 7:              F-B13               Stages 10, 10.5
Chunk 8:              F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9:              F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):    F-B16               Refinement loop
```

### 0.3 Where F-B11 and F-B12 sit in the pipeline

F-B11 and F-B12 are the **two spatial plans** that bracket the boundary
between "what objects need bytes?" and "where exactly do those bytes
live?":

* **F-B11 (Stage 8.5) `OverlayPlan`** is the install/layout plan for every
  WRAM-overlay residency choice F-B10's `RomWindowPlan` made. It owns
  three things: which **regions** of WRAM are reserved for overlays, which
  **share classes** of overlayable objects co-occupy each region, and the
  **install events** (when an install may happen, what it sources, what
  region it targets, which lease shape it consumes). It reserves WRAM
  bytes — counted, but **not yet addressed** — so that F-B12 can honor
  those reservations exactly when arena assignment runs.

* **F-B12 (Stage 9) `ArenaPlan`** is the byte-range allocator. It assigns
  named arenas (ping-pong activations, accum scratch, route scratch,
  decode scratch, continuation record, persistent sequence-state pages,
  trace pages, harness command/result blocks) and concrete byte ranges to
  every value `StoragePlan` materialized (`Materialization::Materialize`
  or `Materialization::Persist`). Pure expression nodes do not get arena
  slots. WRAM regions reserved by `OverlayPlan` are honored byte-for-byte;
  persistent-page geometry follows the SRAM persistence protocol.

These two stages are the **last spatial passes before scheduling**. After
this chunk closes, every materialized value has a deterministic address,
every overlay install has a region and a moment, and every persistent
page maps onto a `PersistPageId` whose byte geometry honors the
double-buffered, commit-grouped record protocol from `planv0.md` §"Persistent
record protocol".

```text
   QuantGraph (F-B3)
        |
        v
   GbInferIR (F-B5)
        |
        +--> ObservationPlan (F-B6)
        |        |
        |        v
        +--> RangePlan (F-B7)
        |        |
        |        v
        +--> StoragePlan (F-B8) ─────────────┐
                    StorageBinding           |    <-- "what is materialized,
                    Materialization          |         what's persisted, what's
                    LifetimeClass            |         recomputed"
                    AliasClassId             |
                                             |
   SramPagePlan (F-B9)  <─────────────  page-shaped persistent values
                                             |
   RomWindowPlan (F-B10)  <───────────── kernel residency choices
        KernelResidency::WramOverlay  ──────────┐
                                             |  |
                                             v  v
                              +-----------------------+
                              | OverlayPlan (F-B11)   |    ← THIS RFC
                              |                       |
                              |   regions             |
                              |   share_classes       |
                              |   installs            |
                              |   wram_overlay_reservation_bytes (counted)
                              |                       |
                              |   emits overlay_plan.json
                              |   (optional certs/overlay.cert.json)
                              +-----------+-----------+
                                          |
                                          v
                              +-----------------------+
                              | ArenaPlan (F-B12)     |    ← THIS RFC
                              |                       |
                              |   wram_arenas         |
                              |   sram_arenas         |
                              |   hram_assignments    |
                              |   overlay_reservation |
                              |     (honors F-B11)    |
                              |                       |
                              |   emits arena_plan.json
                              |        certs/arena.cert.json
                              +-----------+-----------+
                                          |
                                          v
                              GbSchedIR (F-B13) — slices reference
                                                  ArenaSlot; leases
                                                  reference OverlayId
```

### 0.4 Cross-epic interactions

F-B11 + F-B12 sit at the intersection of four epics:

```text
Epic A → Epic B
  - gbf-foundation (Hash256, BlobRef, sized-byte-budget wrappers)  consumed
  - gbf-hw (TargetProfile, MemoryMap regions: WRAM/HRAM/SRAM/ROM)  consumed
  - gbf-abi (PersistHeader, PersistKind, PageState, CommitGroupId,
             InferenceStateHeader + tail liveness window, HarnessCommandBlock,
             HarnessResultBlock layouts)                            consumed
  - gbf-runtime::banking (BankLease/BankGuard ABI; install
             trampolines must conform)                              consumed
  - gbf-runtime::persistence (PersistHeader, page rotation contract;
             arena persistent pages must match the protocol)        consumed
  - gbf-store (StageCache) for K11 / K12 cache wiring               consumed

Epic B (internal):
  - F-B2 / F-B4 ReportEnvelope rule + StageCache convention         inherited
  - F-B3 / F-B5 IR products (consumed only transitively through
                              StoragePlan/RomWindowPlan)              n/a-direct
  - F-B8 StoragePlan products (StorageBinding, Materialization,
                               LifetimeClass, AliasClassId)         consumed
  - F-B9 SramPagePlan products (page bindings, page-switch budgets) consumed
  - F-B10 RomWindowPlan products (KernelResidency, RomWindowBinding,
                                  WRAM-overlay candidate set)       consumed
  - F-B13 GbSchedIR + ResourceStateValidation                       feeds
  - F-B14 ScheduleCostAnalysis (overlay-install cycle cost)         feeds
  - F-B16 FeasibilityRefinementLoop                                 feeds
  - F-B17 StageCache cross-cut                                      compatible

Epic C → Epic B (oracle correspondence):
  - F-C3 ScheduleOracle consumes ArenaPlan named arenas             provided
        (arena geometry must match what ScheduleOracle binds in
         emulator harness mode)

Epic F → Epic B:
  - certs/arena.cert.json is the canonical arena certificate        produced
  - certs/overlay.cert.json (optional) extends the certificate set  produced
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk straddles the front of M3 and
unblocks M4:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring.

M2            One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; first ReachabilityValidation pass.
              ↳ F-B11 closes the M2 commitment that "kernel residency
                 selected by RomWindowPlan" actually has bytes installed
                 from somewhere; without it, WramOverlay is a label
                 rather than a plan.

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B12 is the M3-commitment delivery: ArenaPlan is what
                makes "wired end-to-end" mean "every materialized value
                has a typed, deterministic address." Without it, the
                cooperative scheduler has no concrete byte map.

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ F-B12 lands the persistent-page byte geometry that the
                M4+ sequence-state work will fill. The arena slots for
                continuation, persistent sequence-state pages, trace
                pages, and harness blocks are pinned now; the persistence
                producer/consumer logic lives in Epic D.
```

### 0.6 What the project as a whole gains when this chunk lands

```text
1. Bytes finally have addresses.
   F-B12 is the first stage that turns "this value is materialized" into
   a concrete WRAM/SRAM/HRAM byte range. After this, every later report,
   diagnostic, and certificate can resolve a value to an address.

2. WramOverlay becomes a real plan, not a residency tag.
   F-B11 turns RomWindowPlan's WramOverlay residency into install events
   with regions, share classes, and reserved bytes. Without it, F-B10's
   residency choice has no executable consequence.

3. The reservation handshake is uniform.
   F-B11 reserves WRAM bytes (counted but not addressed); F-B12 honors
   those reservations exactly. This is the load-bearing precondition for
   F-B13 slice scheduling: a slice cannot reference an ArenaSlot whose
   WRAM region was implicitly stolen by an overlay install.

4. F-B13 (GbSchedIR) becomes implementable.
   Slices reference ArenaSlot; leases reference OverlayId. Without F-B12
   there is no slot to reference; without F-B11 there is no overlay id.

5. F-C3 (ScheduleOracle) becomes implementable.
   Named arenas are the storage geometry the schedule oracle must reproduce
   in emulator harness mode. Once the arena names and byte ranges are
   fixed, ScheduleOracle has a concrete contract to bind against.

6. The persistent-record protocol gets its compile-time witness.
   The SRAM persistence protocol (planv0 §"Persistent record protocol")
   says pages have headers, double-buffering, generation counters, and
   commit-group manifests. F-B12 is what writes those bytes into the
   arena map. The runtime later validates against the same shape.

7. certs/arena.cert.json is canonical.
   The first stage that produces a numbered certificate from this chunk's
   work. After F-B12, the certs/ directory has a typed commitment to
   "every materialized value's address is honest, alias-correct, and
   reservation-honoring."

8. Reservation accounting is auditable.
   OverlayPlan reservations are first-class, byte-exact, and honored.
   Over-reservation, under-reservation, and double-counting are all
   typed rejection classes — not silent bugs.
```

### 0.7 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later stage receives a typed `OverlayPlan` and a typed `ArenaPlan`
  whose products are content-addressed. F-B13 never invents an
  `ArenaSlot`; F-B14 never re-derives overlay-install cost from `KernelResidency`
  alone.
* WRAM, HRAM, and SRAM byte budgets are partitioned into named arenas
  and reservations exactly once. No later stage may carve a new arena
  or steal from an existing one without amendment.
* Persistent-page byte geometry matches the SRAM persistence protocol's
  page header + commit-group layout. Any later stage that wants to write
  a `PersistKind` page consumes the byte range from `ArenaPlan`.
* `OverlayId`, `ArenaId`, and `ArenaSlot` names are stable across runs.
  Symbol map, listing, and `.sym` output (Epic A) consume them.

### 0.8 Reading order for reviewers

```text
§0  (this section) — placement and dependencies
§0a TL;DR
§1  Project context — milestone-specific framing
§2  Load-bearing decisions — the engineering choices that bracket the rest
§5  Authority rules — what this RFC owns vs inherits
§6  Pipeline state machine — how Stage 8.5 and Stage 9 plug in
§8  Stage 8.5 contract: OverlayPlan
§9  Stage 9 contract: ArenaPlan
§10 Address invariants
§11 Reservation accounting
§12 Report schemas (overlay_plan.v1, arena_plan.v1, certs/arena.cert.v1)
§16 Task DAG
§19 End-to-end theorem
§20 Final concise contract
```

Skim §3, §4, §7, §13, §14, §15, §17, §18 for specifics.

## 0a. TL;DR

This chunk lands the **two spatial plans** that bracket the boundary
between "what objects exist?" and "where exactly do they live?". It owns
two numbered stages:

* **Stage 8.5 — `OverlayPlan`.** For every kernel/LUT/expert-fragment that
  `RomWindowPlan` (F-B10) marked `KernelResidency::WramOverlay`, decide
  which **region** of WRAM hosts it, which other overlayables it **shares**
  that region with (sequenced installs over the same bytes), and **when**
  installs may occur. Reserve WRAM bytes — counted but not addressed —
  so `ArenaPlan` honors the reservation exactly.

* **Stage 9 — `ArenaPlan`.** Assign **named arenas** and **concrete byte
  ranges** to every value that `StoragePlan` (F-B8) marked
  `Materialization::Materialize { class, lifetime }` or
  `Materialization::Persist { page, commit_group }`. Honor `OverlayPlan`'s
  WRAM reservation byte-for-byte. Persistent pages match the SRAM
  persistence protocol's geometry (header, double-buffered pages,
  commit-group manifest). Pure expression nodes get **no** slot.

These two features are paired in one RFC because they share the
**spatial plan** shape: each is a typed transform from pinned upstream
products into a content-addressed plan, each emits a canonical JSON
report, each is consumed by F-B13 (`GbSchedIR`) by hash, each is wrapped
in the `FeasibilityRefinementLoop` (F-B16), and each shares the diagnostic
envelope, JSON canonicalization rule, self-hash convention, and
`StageCache` key construction inherited from F-B2/F-B4.

The chunk closes only when:

1. `OverlayPlan` construction is a deterministic pure function of the
   pinned upstream products (`StoragePlanProduct`, `SramPagePlanProduct`,
   `RomWindowPlanProduct`, `ResolvedCompilePolicy`, `RuntimeChromeBudget`)
   and is byte-identical across two consecutive regenerations.
2. `ArenaPlan` construction is a deterministic pure function of
   (`StoragePlanProduct`, `SramPagePlanProduct`, `RomWindowPlanProduct`,
   `OverlayPlanProduct`, `ResolvedCompilePolicy`, `RuntimeChromeBudget`)
   and is byte-identical across two consecutive regenerations.
3. `overlay_plan.json` round-trips through its semantic validator and
   self-hash. (Optional `certs/overlay.cert.json` is added as an "Amends
   planv0" in §12.4 if the certificate sharpens the contract.)
4. `arena_plan.json` and `certs/arena.cert.json` round-trip through their
   semantic validators and self-hashes.
5. Every `WramOverlay` kernel from `RomWindowPlan` has at least one
   `OverlayInstall`; every `OverlayInstall` references an `OverlayId` that
   exists; every `OverlayShareClass` member fits the same region's byte
   budget; the sum of region reservations does not exceed
   `RuntimeChromeBudget.wram_overlay_cap_bytes`.
6. Every `Materialize` binding from `StoragePlan` has exactly one
   `ArenaSlot`; every `Persist` binding maps to exactly one
   `(PersistPageId, byte_range)`; ranges do not overlap within an arena
   except where alias-class equivalence permits; reservations from
   `OverlayPlan` are honored exactly (no overflow, no underflow).
7. `LifetimeClass` is preserved from `StoragePlan` to `ArenaSlot`. Bank0
   budgets honor `RuntimeChromeBudget.bank0_*` caps. Harness command/result
   blocks live in the SRAM `Harness` arena and never leak into model
   arenas.
8. `StageCache` keys K11 (Stage 8.5) and K12 (Stage 9) are pinned and
   tested.

The chunk does **not** include:

* Slice scheduling — owned by F-B13 (Stage 10).
* Lease tracking and `ResourceStateValidation` — owned by F-B13 (Stage
  10.5). `OverlayId` is a name; the lease lifecycle that consumes it is
  F-B13's job.
* Codegen — owned by F-B15 (Stage 12).
* Far-call legalization or section ordering — owned by F-B15.
* Refinement-loop repairs — owned by F-B16.
* Persistence producer/consumer logic — owned by Epic D.
* Trace data production — owned by F-B14 / Epic D.
* `PersistHeader` / `PersistGroupCommit` runtime mutation — owned by
  `gbf-runtime::persistence`.
* `BankLease`/`BankGuard` runtime — owned by F-A4. F-B11/F-B12 produce
  data plans whose lease shape is consumed by F-B13.

## 1. Project context — where these stages sit in the milestone sequence

### 1.1 What F-B2 / F-B3 / F-B4 / F-B5 / F-B6 / F-B7 / F-B8 / F-B9 / F-B10 leave on the table

By the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, calibration, hint bundle, and
  `CompileRequest` are admissible and hash-bound through
  `artifact_validation.json` (F-B2).
* `ResolvedCompilePolicy` is the single answer to "what policy governed
  this build" with provenance for every load-bearing scalar (F-B2).
* `RuntimeChromeBudget` has been honored at the static byte-math level
  (F-B4) with a successful `static_budget.json`.
* `QuantGraph` (F-B3) and `GbInferIR` (F-B5) are content-addressed and
  storage-free.
* `ObservationPlan` (F-B6), `RangePlan` (F-B7) bind probes/checkpoints
  and reduction structure.
* `StoragePlan` (F-B8) has decided, for every `ValueId` in `GbInferIR`,
  whether the value is `Recompute`, `Materialize { class, lifetime }`, or
  `Persist { page, commit_group }`, plus its `AliasClassId`.
* `SramPagePlan` (F-B9) has assigned page-state geometry, page rotation,
  spill policy, and commit boundaries to every `Persist` binding whose
  `class` is `SramPaged`.
* `RomWindowPlan` (F-B10) has resolved kernel and LUT residency:
  `Bank0Fixed`, `WramOverlay`, or `CoResidentSwitchable`. The
  `WramOverlay` choices are the input to `OverlayPlan`.

What is *not* yet decided when this chunk begins:

* No object has been given a concrete byte range. Materialization is
  decided; addresses are not.
* No `WramOverlay` kernel has been told *where* in WRAM it lives or
  *when* it is installed.
* No persistent page has been mapped onto a byte range that satisfies
  the `PersistHeader` + double-buffered + commit-group geometry.
* No reservation accounting links F-B11's reservations to F-B12's arenas.

This chunk is responsible for closing those gaps deterministically and
auditably.

### 1.2 Why these two stages are paired

The natural unit is "the two spatial passes that bracket the boundary
between materialization and addressing."

* If we made it one feature, the bead would carry both an overlay
  install/share planner and a multi-arena byte-range allocator. PR review
  would fragment, and `OverlayPlan`'s reservation semantics would get
  conflated with `ArenaPlan`'s honoring rule. Worse, the reservation
  handshake is the point of the boundary; collapsing the two stages
  hides it.
* If we made it three features (e.g. F-B11a regions, F-B11b installs,
  F-B12 arenas), we would split on internal structure that re-converges
  at codegen. The reservation handshake remains between F-B11 and F-B12
  in any split.
* Two features matches the natural seam: `OverlayPlan` owns reservation
  + install/share semantics; `ArenaPlan` owns named-arena addressing +
  persistent-page geometry. They are paired in this RFC because:
  (a) `ArenaPlan` cannot run without `OverlayPlan`'s reservation;
  (b) both consume the same upstream products (`StoragePlan`,
  `RomWindowPlan`, `SramPagePlan`, `ResolvedCompilePolicy`,
  `RuntimeChromeBudget`);
  (c) both share the F-B2/F-B4 inherited surfaces (envelope, JSON,
  self-hash, StageCache);
  (d) both feed F-B13 in lockstep.

### 1.3 What this chunk retires for the rest of Epic B

By the time the next chunks begin:

* Every later stage receives a typed `OverlayPlanProduct` and a typed
  `ArenaPlanProduct` whose self-hashes pin every load-bearing scalar.
* F-B13 (`GbSchedIR`) consumes `ArenaSlot` ids verbatim. It never
  carves a new slot or moves bytes.
* F-B13's `ResourceStateValidation` consumes `OverlayId` and validates
  lease balance against `OverlayPlan`'s install events.
* F-B14 (`ScheduleCostAnalysis`) charges overlay installs against the
  cycle calibration, but does not select which kernels overlay.
* F-B15 (Backend) emits sections that match `ArenaPlan` byte ranges.
  Section ordering is F-B15's; address ranges are F-B12's.
* F-C3 (`ScheduleOracle`) binds emulator harness state to the named
  arenas in `ArenaPlan`. The geometry is fixed here.

### 1.4 What this chunk is NOT

The chunk is medium in scope but very large in contract surface. To
prevent scope creep:

* It is **not** a slice scheduler. F-B11 declares which install events
  may occur; F-B13 decides when they occur within a slice.
* It is **not** a lease tracker. `OverlayId` is a name; the lease
  lifecycle that consumes it is F-B13's `ResourceStateValidation` job.
* It is **not** a codegen pass. F-B15 emits the bytes; F-B12 just owns
  the addresses.
* It is **not** a far-call legalizer or section orderer. F-B15 does
  branch relaxation, far-call thunk insertion, and bank-switch coalescing
  against the addresses F-B12 produces.
* It is **not** the producer of the persistent record protocol. The
  protocol lives in `gbf-abi` and `gbf-runtime::persistence` (F-A4,
  Epic D). F-B12 lays out byte ranges that conform to the geometry; the
  producer/consumer logic lives at runtime.
* It is **not** an overlay loader. Overlay byte transport from ROM to
  WRAM is a runtime kernel emitted by F-A5 / F-B15 with the F-A4
  `BankLease` ABI. F-B11 records the install events; the runtime
  performs them.
* It is **not** a refinement loop. `OverlayPlan` and `ArenaPlan` are
  immutable products of their stages; `RepairPolicy` and `CompileKnobs`
  drive future runs through F-B16.
* It does **not** mutate the upstream products. `OverlayPlan` does not
  rewrite `KernelResidency`; `ArenaPlan` does not change
  `StorageBinding.materialization`.

## 2. Load-bearing decisions

### 2.1 Pure-function shape (core / driver split)

Both stages have **two layers**: a pure core constructor and a thin
driver that performs IO. The core is a pure function from typed pinned
inputs to typed content-addressed products. The driver wraps the core
with JSON emission and StageCache writes.

```text
build_overlay_plan_core(OverlayPlanInputs)
  -> Result<(OverlayPlan, ReportEnvelope<OverlayPlanReportBody>),
            PassDiagnostics>

run_stage8_5(OverlayPlanInputs, env)
  = build_overlay_plan_core(...) then
    (on success or failure):
      emit overlay_plan.json
      may emit certs/overlay.cert.json
      may write StageCache success entry
      may write StageCache failure memo

build_arena_plan_core(ArenaPlanInputs)
  -> Result<(ArenaPlan, ReportEnvelope<ArenaPlanReportBody>),
            PassDiagnostics>

run_stage9(ArenaPlanInputs, env)
  = build_arena_plan_core(...) then
    (on success or failure):
      emit arena_plan.json
      emit certs/arena.cert.json
      may write StageCache success entry
      may write StageCache failure memo
```

Cores never mutate `StoragePlan`, `RomWindowPlan`, `SramPagePlan`,
`ResolvedCompilePolicy`, `RuntimeChromeBudget`, or `OverlayPlan`.
Drivers are the only IO surface. Determinism is required, not
aspirational.

The chunk-level pass shape is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed reservation accounting)
       (typed byte-range derivation)
       (typed alias / lifetime preservation)
       (typed persistent-page geometry binding)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed plan product, ReportEnvelope<ReportV1> }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON
       emits cert (Stage 9 always; Stage 8.5 optional)
       writes StageCache success / failure memo
```

Every report includes `outcome: ReportOutcome` per F-B2/F-B4 §2.1.

### 2.2 Inheritance from F-B2/F-B4 and F-B3/F-B5

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
  §14 of this RFC; they extend the closed enum without modifying
  existing variants.
* `D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`, `D-Provenance`
  diagnostic laws — F-B2/F-B4 §5.
* `R-NoPartialProduct`: failed reports have `body.result = None`.
  F-B11/F-B12 reports MUST NOT contain a partial plan — F-B3/F-B5 §7.
* StageCache key construction rule
  `DomainHash(crate, "StageCacheKey", schema_id, schema_version, canonical_json_bytes)`
  — F-B2/F-B4 §11.
* StageCache success/failure-memo cache laws — F-B2/F-B4 §2.6.

If a later amendment to F-B2/F-B4 or F-B3/F-B5 changes any of the above,
that amendment must explicitly amend this RFC by name.

This RFC adds the following to that surface:

* Two new `ValidationOrigin` variants: `OverlayPlanConstruction` and
  `ArenaPlanConstruction`.
* Two new `ReportSchemaId` variants: `overlay_plan.v1` and
  `arena_plan.v1`. One new optional certificate schema
  `arena.cert.v1` (and an "Amends planv0" optional `overlay.cert.v1`).
* Two new public plan product types: `OverlayPlan` and `ArenaPlan`.
* Two new public report bodies: `OverlayPlanReportBody` (§12.1),
  `ArenaPlanReportBody` (§12.2). One certificate body
  `ArenaCertBody` (§12.3) and an optional `OverlayCertBody` (§12.4).
* Two new `StageCacheKey` schemas (§13): `K11 := OverlayPlanCacheKey`,
  `K12 := ArenaPlanCacheKey`.

### 2.3 Region-first overlay design

`OverlayPlan` is **region-first**: regions own bytes; installs own time;
share classes own region-sharing equivalence.

```text
OverlayPlan ::= ( regions, share_classes, installs )

regions[i] = OverlayRegion {
    id: OverlayId,
    bytes: u16,
    constraint: WramRegionConstraint,
    members: NonEmptyList<OverlayResidentId>,
    reservation_kind: ReservationKind::WramOverlay,
    reservation_floor_bytes: u16,
    reservation_ceil_bytes:  u16,
}
```

Bytes belong to regions. **No** install holds bytes that the region does
not already own. Two installs that target the same region time-share the
bytes; they are members of the same `OverlayShareClass`.

```text
share_classes[k] = OverlayShareClass {
    id: ShareClassId,
    region: OverlayId,
    members: NonEmptyList<OverlayResidentId>,
    eviction: EvictionPolicy,
}
```

Members of one share class fit the same region (their max payload size
must be ≤ `region.bytes`). Eviction policy decides which member is
resident at any moment; the runtime executes installs at the moments
F-B13 schedules.

```text
installs[m] = OverlayInstall {
    id: InstallId,
    region: OverlayId,
    member: OverlayResidentId,
    source: OverlaySource,                 // ROM bank + offset; LUT id; etc.
    install_event: OverlayInstallEvent,
    lease_shape: OverlayLeaseShape,
}
```

`OverlayInstallEvent` declares **when** an install may occur — at slice
entry, at slice exit, at a phase boundary, etc. F-B13 picks the actual
moments.

This region/install/share-class triad is the closed shape; new shapes
require RFC amendment.

### 2.4 Arena naming as global vocabulary

`ArenaPlan` uses a **named-arena enum** with a closed set of variants in
v1:

```rust
pub enum NamedArena {
    // WRAM arenas
    WramActivationsPingA,
    WramActivationsPingB,
    WramAccumScratch,
    WramRouteScratch,
    WramDecodeScratch,
    WramContinuationRecord,
    WramOverlayRegion(OverlayId),

    // SRAM arenas
    SramSequenceStatePages(SequenceStreamId),
    SramTracePages,
    SramHarnessCommandBlock,
    SramHarnessResultBlock,
    SramPersistedTranscript,
    SramColdSpill,

    // HRAM assignments (single-byte or small-byte)
    HramFrameFlags,
    HramBankShadow,
    HramFaultCode,
    HramSchedulerScratch,
    HramYieldRequested,
}
```

Names are stable across runs. Symbol-map output (`gbf-asm::symbols`)
consumes these names verbatim. Any new arena class requires RFC amendment
plus a `NamedArena` enum bump.

### 2.5 Byte-range determinism

Within a chosen `NamedArena`, byte ranges are assigned by a deterministic
**first-fit-decreasing** algorithm with a typed tie-break:

```text
FFD-Order(slot_a, slot_b):
  primary: descending slot.size_bytes
  secondary: ascending slot.alias_class_id
  tertiary: ascending slot.lifetime_class_priority
  quaternary: ascending slot.value_id

If the bin is empty, place at offset 0.
If the bin has free intervals, place in the first interval that fits;
  recurse on the remaining free intervals after placement.
```

The algorithm is documented in §9.4. It is deterministic across machines,
across runs, and across cargo features (the feature set hash is part of
`K12`).

### 2.6 Lifetime-class-driven arena selection

Each `LifetimeClass` from `StoragePlan` has a fixed mapping to allowed
arena families:

| `LifetimeClass`    | Allowed `NamedArena` family                                                   |
|--------------------|-------------------------------------------------------------------------------|
| `Slice`            | `WramAccumScratch`, `WramRouteScratch`, `WramDecodeScratch`                   |
| `ResumeWindow`     | `WramActivationsPingA`, `WramActivationsPingB`                                |
| `Token`            | `WramActivationsPingA`, `WramActivationsPingB`, `WramAccumScratch`            |
| `Session`          | `WramContinuationRecord`, `SramPersistedTranscript`                           |
| `Persistent`       | `SramSequenceStatePages(_)` (must be `Persist { page, commit_group }`)        |

A binding whose `LifetimeClass` does not match any allowed family for its
`StorageClass` is a hard reject (`ArenaLifetimeClassMismatch`). The
mapping is closed and pinned; new lifetime classes or arena families
require RFC amendment.

### 2.7 Persistent-page byte layout under the SRAM persistence protocol

Every `Materialization::Persist { page: PersistPageId, commit_group }`
binding maps onto an SRAM byte range whose layout matches
`gbf-runtime::persistence`'s page geometry:

```text
page_byte_range(p) = (header_range_p, payload_range_p, commit_marker_p)

  header_range_p:    sizeof(PersistHeader)        // start of page
  payload_range_p:   immediately after header     // page-class payload
  commit_marker_p:   last 2 bytes of page         // commit word

Pages of the same kind (e.g. SequenceState) are double-buffered as a pair
(page A, page B); the active page is selected by the commit-group manifest
(PersistGroupCommit) elsewhere in the same arena.
```

The `PersistHeader` shape (per `planv0.md` line 2143) and
`PersistGroupCommit` (line 2157) are runtime-owned `#[repr(C)]` types in
`gbf-abi`. F-B12 produces byte ranges that match their `size_of` and
alignment. F-B12 does not write the bytes; it lays out where they go.

### 2.8 Aliasing and lifetime preservation from StoragePlan

`StoragePlan` produces `StorageBinding { value, materialization, alias_class }`.
F-B12 preserves both:

* Two bindings with the same `AliasClassId` may share an `ArenaSlot`'s
  byte range when their `LifetimeClass` permits (specifically, when their
  lifetimes are non-overlapping, both are `Slice`-class, or the alias
  class is declared `MustOverlap`).
* Two bindings with different `AliasClassId` may **never** share a byte
  range, regardless of lifetime.
* `LifetimeClass` is preserved verbatim from `StorageBinding` to
  `ArenaSlot`. `ArenaSlot.lifetime_class = StorageBinding.materialization.lifetime`
  for `Materialize` bindings; `Persist` bindings derive lifetime from
  `LifetimeClass::Persistent`.

### 2.9 Reservations are first-class accounting

`OverlayPlan` reserves WRAM bytes; `ArenaPlan` honors them. The
reservation contract is explicit:

```text
F-Reservation-Defn:
  reserved_bytes = sum over r in OverlayPlan.regions of r.bytes
  reserved_bytes <= RuntimeChromeBudget.wram_overlay_cap_bytes

F-Reservation-Honored:
  ArenaPlan.overlay_reservation.total_bytes = OverlayPlan.reserved_bytes
  ∀ r ∈ OverlayPlan.regions.
    ∃! a ∈ ArenaPlan.wram_arenas with a.named = WramOverlayRegion(r.id)
                                     ∧ a.size_bytes = r.bytes
                                     ∧ a.byte_range disjoint from every
                                       non-overlay WRAM arena.
```

Honoring is **exact**:

```text
F-Reservation-NoOverflow:
  ∀ r ∈ OverlayPlan.regions.
    ArenaPlan.wram_arenas.lookup(WramOverlayRegion(r.id)).size_bytes <= r.bytes

F-Reservation-NoUnderflow:
  ∀ r ∈ OverlayPlan.regions.
    ArenaPlan.wram_arenas.lookup(WramOverlayRegion(r.id)).size_bytes >= r.bytes

  Equivalently: size_bytes == r.bytes.

F-Reservation-Disjoint:
  ∀ r ∈ OverlayPlan.regions, ∀ a ∈ ArenaPlan.wram_arenas with
      a.named ≠ WramOverlayRegion(r.id).
    ArenaPlan.wram_arenas.lookup(WramOverlayRegion(r.id)).byte_range
      does not intersect a.byte_range.
```

Under-reservation is permitted at the *plan* level: `OverlayPlan` may
reserve fewer bytes than `RuntimeChromeBudget.wram_overlay_cap_bytes`
allows (the unused budget is left for future overlays). Over-reservation
is a hard reject (`OverlayWramOverlayCapExceeded`).

`ArenaPlan` over-using a reservation (`ArenaOverlayReservationOverflow`)
or under-using a reservation (`ArenaOverlayReservationUnderflow`) are
both hard rejects. The size-equality rule ensures that the runtime sees
exactly the bytes the plan promised.

Amends planv0: planv0.md line 1745 says `OverlayPlan` "decides ... what
WRAM budget must be reserved for overlays before arena assignment
begins." This RFC pins the reservation as **byte-exact** and elevates
over/under to typed rejection classes.

### 2.10 Bank0 budgets are honored, not produced

F-A5 (Bank0 runtime nucleus) and `RuntimeChromeBudget` (F-B4-consumed)
declare the Bank0 byte budget for runtime, vectors, ISR, scheduler,
banking ABI, panic, harness entry, and far-call trampolines. F-B11/F-B12
do not allocate Bank0 ROM bytes — F-B15's PlacedRom does — but Bank0
**WRAM** and **HRAM** are partially F-B12's:

* `WramContinuationRecord` starts with the fixed
  `gbf-abi::InferenceStateHeader` (`#[repr(C)]`) and extends by the
  session continuation tail window required for materialized session
  state.
* `HramFrameFlags`, `HramBankShadow`, `HramFaultCode`,
  `HramSchedulerScratch`, `HramYieldRequested` are pinned HRAM
  assignments whose byte addresses are stable across runs.

F-B12 records these as input-bound facts; their sizes come from
`gbf-abi`'s `#[repr(C)]` types and are validated against
`RuntimeChromeBudget.hram_usable_bytes` and `RuntimeChromeBudget.wram_runtime_floor_bytes`.

### 2.11 No SRAM page spans, no harness leaks

```text
F-Sram-NoSpan:
  ∀ slot ∈ ArenaPlan.sram_arenas.
    slot.byte_range fits entirely within one 8 KiB SRAM bank window
    [base, base + 8192).

F-Sram-PersistGeometry:
  ∀ slot whose named ∈ {SramSequenceStatePages(_)}.
    slot.byte_range layout matches PersistHeader + payload + commit_word
    geometry per §2.7 and the runtime persistence protocol.

F-Sram-NoHarnessLeak:
  ∀ slot whose named ∈ {SramHarnessCommandBlock, SramHarnessResultBlock}.
    slot.byte_range is disjoint from every slot whose named is one of
    {SramSequenceStatePages(_), SramPersistedTranscript, SramColdSpill,
     SramTracePages}.
```

The harness arena lives in a dedicated SRAM page family so a crash or a
malformed harness write cannot contaminate sequence-state recovery
(`planv0.md` line 2204).

### 2.12 Determinism and cache discipline

Both plan products are fully content-addressed:

```text
overlay_plan_self_hash := DomainHash(
    "gbf-codegen", "OverlayPlan", "v1",
    CanonicalJson(OverlayPlan after canonical sort))

arena_plan_self_hash := DomainHash(
    "gbf-codegen", "ArenaPlan", "v1",
    CanonicalJson(ArenaPlan after canonical sort))
```

Determinism axioms:

```text
F-Det-Overlay:
  Same StoragePlanProduct + SramPagePlanProduct + RomWindowPlanProduct
       + ResolvedCompilePolicy + RuntimeChromeBudget
  ⇒ byte-identical OverlayPlan and overlay_plan.json.

F-Det-Arena:
  Same StoragePlanProduct + SramPagePlanProduct + RomWindowPlanProduct
       + OverlayPlanProduct + ResolvedCompilePolicy + RuntimeChromeBudget
  ⇒ byte-identical ArenaPlan and arena_plan.json.
```

`StageCache` keys (K11, K12) participate in the determinism witness:
two builds with identical input hashes hit the cache; one byte changed
in any input misses.

### 2.13 No scheduling, no leases

F-B11 and F-B12 produce **spatial** plans only. Slice ordering, lease
acquisition/release, interrupt policy, and yield safety are all F-B13
(`GbSchedIR + ResourceStateValidation`).

```text
F-NoScheduling:
  OverlayPlan and ArenaPlan contain no SliceId, LeaseId, ResourceVector,
  CycleBudget, YieldKind, ExitKind, or any scheduling field.

F-OverlayLease:
  OverlayInstall.lease_shape is a static descriptor (e.g.
  "RomBank lease for source bank during install"), not a live LeaseId.
  F-B13 mints LeaseIds against this shape.
```

### 2.14 No section ordering, no codegen

```text
F-NoSection:
  ArenaPlan does not order or place sections. Section roles, far-call
  thunk insertion, branch relaxation, and bank placement are F-B15's job.

F-NoCodegen:
  Neither plan emits AsmIR, byte sequences, or pseudo-ops.
```

### 2.15 Repair policy is named-only

```text
F-NoRepairInChunk:
  No diagnostic in overlay_plan.json or arena_plan.json carries a
  RepairProposal source or any AuthorizedRelaxation operation.
  PolicySource ⊆ {TargetDefault, ProfileDefault, CompileRequestOverride,
                  HintBundle, Calibration} (per F-B2 §2.7).

F-RepairKnobsWired:
  CompileKnobs schema (resolved by F-B2 Stage 0.5) carries any
  overlay-/arena-relevant knobs as named-only hooks. F-B16 unblocks
  RepairProposal source. No knob is *consumed* by F-B11 or F-B12 in v1
  beyond reading the resolved value.
```

### 2.16 No "quick fix" defaults

If `OverlayPlan` would only succeed by silently filling in a default
region size, eviction policy, or install event, it fails. If `ArenaPlan`
would only succeed by silently picking an arena for a binding whose
mapping is ambiguous, it fails. Every plan field derives from a
hash-bound input or fails loudly.

### 2.17 Single placement model in v1

For the chunk-closure fixture, `OverlayPlan` v1 emits **one** region per
build (`regions.len() == 1`) and **no** share classes; share-class
support is in the schema but exercised only in fixture builds. This
mirrors the bd-140 BringUp-v1 plan. The schema is forward-compatible:
later builds may emit multiple regions and multiple share classes
without an RFC amendment, provided the `OverlayShareClass` constraint is
satisfied.

Amends planv0: planv0.md line 1747 leaves `OverlayPlan.regions` plural
without specifying v1 cardinality. This RFC pins v1 to "regions ≥ 1,
share_classes may be empty" while keeping the schema plural.

### 2.18 Schema versioning

```text
overlay_plan.v1
arena_plan.v1
arena.cert.v1
overlay.cert.v1     (optional; "Amends planv0" — see §12.4)
```

Schema bumps follow F-B2/F-B4 §10's compatibility rules.

## 3. Glossary additions

This chunk introduces or pins the following terms beyond the F-B2/F-B4
and F-B3/F-B5 glossary inheritance.

| Term                       | Definition                                                                                  |
|----------------------------|---------------------------------------------------------------------------------------------|
| Spatial plan               | A plan that assigns concrete WRAM/SRAM/HRAM byte ranges or reserves them for later assignment. |
| OverlayId                  | Region identifier in `OverlayPlan`. Stable across runs. Referenced by `ArenaPlan.wram_arenas` and by `ResourceLeaseKind::Overlay`. |
| OverlayResidentId          | Object identity for a member of an overlay region (a kernel, a LUT fragment, an expert micro-fragment). |
| ShareClassId               | Equivalence class identifier for overlayables that may time-share one region. |
| InstallId                  | Identifier for one `OverlayInstall` event. |
| OverlayInstallEvent        | Static descriptor of when an install may occur (slice entry, phase boundary, etc.). |
| OverlayLeaseShape          | Static descriptor of the lease(s) an install requires. F-B13 mints concrete `LeaseId`s. |
| Reservation                | Counted-but-not-addressed WRAM bytes owned by `OverlayPlan` and honored byte-for-byte by `ArenaPlan`. |
| NamedArena                 | Closed enum of arenas in `ArenaPlan`. v1 set is in §2.4. |
| ArenaId                    | Identifier for one named arena. Stable across runs. |
| ArenaSlot                  | A `(byte_range, alias_class, lifetime_class, value_id?)` record inside an arena. |
| ArenaBindings              | The map `StorageBinding -> ArenaSlot` for `Materialize` bindings; the map `(PersistPageId) -> ArenaSlot` for `Persist` bindings. |
| Persistent-page geometry   | The `PersistHeader + payload + commit_word` byte layout that every `SramSequenceStatePages` slot must satisfy. |
| Commit-group               | A set of `Persist` pages that must be mutually consistent. `CommitGroupId` is preserved from `StoragePlan`. |

## 4. Core notation

This RFC inherits §1 of F-B2/F-B4 and §4 of F-B3/F-B5 (Hash256, Outcome,
Severity, Stage, ReportSchema, Result, Option, NonEmptyList, SortedBy,
DomainHash, SelfHash, CanonicalJson, ZERO_HASH, null policy, ValidationOrigin
extensions). Additions:

```text
Stage :=
  Stage0 | Stage0_5 | Stage1 | Stage2 | Stage3 | Stage4 | Stage5
  | Stage6 | Stage7 | Stage8
  | Stage8_5      -- new (OverlayPlan)
  | Stage9        -- new (ArenaPlan)

ReportSchema :=
  ...existing schemas...
  | overlay_plan.v1
  | arena_plan.v1
  | arena.cert.v1
  | overlay.cert.v1                      -- optional, "Amends planv0"

ValidationOrigin (extension) :=
  ...existing F-B2/F-B4/F-B3/F-B5 origins...
  | OverlayPlanConstruction
  | ArenaPlanConstruction
```

Abbreviations used throughout:

```text
OP  := OverlayPlan
AP  := ArenaPlan
SP  := StoragePlan      (F-B8)
RWP := RomWindowPlan    (F-B10)
SPP := SramPagePlan     (F-B9)
RCB := RuntimeChromeBudget
```

## 5. Authority rules

```text
Scope(F-B11/F-B12) =
  {
    Stage8_5,
    Stage9,
    OverlayPlan,
    ArenaPlan,
    overlay_plan.v1,
    arena_plan.v1,
    arena.cert.v1,
    overlay.cert.v1 (optional),
    StageCache keys for Stage8_5 and Stage9,
    reservation accounting between OverlayPlan and ArenaPlan,
    NamedArena enum (v1 closed set),
    persistent-page geometry binding on the arena side,
    LifetimeClass arena-family mapping,
    AliasClassId arena-slot sharing rule
  }

Rule Authority:
  ∀ behavior b.
    b ∈ Scope(F-B11/F-B12) ∧ RFC specifies b
    ⇒ SourceOfTruth(b) = RFC

Rule PlanContext:
  ∀ behavior b.
    b ∈ Scope(F-B11/F-B12) ∧ RFC silent on b
    ⇒ planv0 may inform implementation but is not an acceptance gate

Rule Inheritance:
  ∀ behavior b.
    b ∈ Scope(F-B2/F-B4) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B2/F-B4 RFC

  ∀ behavior b.
    b ∈ Scope(F-B3/F-B5) ∧ b is not amended by this RFC
    ⇒ SourceOfTruth(b) = F-B3/F-B5 RFC

Rule Amendment:
  LaterRFC changes any of:
    public OverlayPlan type
    public ArenaPlan type
    NamedArena enum
    report shape (overlay_plan.v1, arena_plan.v1)
    cert shape (arena.cert.v1, overlay.cert.v1)
    cache key (K11, K12)
    diagnostic code introduced here
    reservation accounting rule
    persistent-page geometry rule
  ⇒ LaterRFC must explicitly amend this RFC

Rule DivergenceLedger:
  RFC intentionally diverges from planv0
  ⇒ nearest relevant section must contain `Amends planv0`
```

## 6. Pipeline state machine

Extending the F-B2/F-B4 / F-B3/F-B5 state machine (and the F-B6/F-B7/F-B8/F-B9/F-B10
chunks' state machines, which this RFC inherits):

```text
State :=
  ...prior states...
  | StoragePlanReady(storage_plan_product, ...)                  -- after F-B8
  | SramPagePlanReady(spp_product, ...)                          -- after F-B9
  | RomWindowPlanReady(rwp_product, ...)                         -- after F-B10
  | OverlayPlanReady(overlay_plan_product, ...)                  -- new
  | ArenaPlanReady(arena_plan_product, ...)                      -- new
  | Halted(stage, report, diagnostics)
```

Transitions (additions to F-B2/F-B4 §6 and prior chunks):

```text
T8_5 build_overlay_plan:
  RomWindowPlanReady(rwp) ∧ SramPagePlanReady(spp) ∧ StoragePlanReady(sp)
    -- build_overlay_plan(rwp, spp, sp, p, rcb) = Ok(op) -->
  OverlayPlanReady(op)

  RomWindowPlanReady(rwp) ∧ ...
    -- build_overlay_plan(...) = Err(e) -->
  Halted(Stage8_5, e.report, e.diagnostics)

T9 build_arena_plan:
  OverlayPlanReady(op) ∧ ...
    -- build_arena_plan(op, rwp, spp, sp, p, rcb) = Ok(ap) -->
  ArenaPlanReady(ap)

  OverlayPlanReady(op) ∧ ...
    -- build_arena_plan(...) = Err(e) -->
  Halted(Stage9, e.report, e.diagnostics)
```

Pipeline invariants (additions):

```text
I-Pipeline-OP-1:
  Stage8_5 may run only after Stage8 (RomWindowPlan) Passed.

I-Pipeline-OP-2:
  Stage9 may run only after Stage8_5 (OverlayPlan) Passed.

I-Pipeline-OP-3:
  If Stage8_5 fails, Stage9 does not run.

I-Pipeline-OP-4:
  Stage8_5 and Stage9 are passive in the plan-product sense:
    They produce their own product but never mutate StoragePlan,
    SramPagePlan, RomWindowPlan, ResolvedCompilePolicy, or
    RuntimeChromeBudget.

I-Pipeline-OP-5:
  overlay_plan.report_self_hash is immutable after Stage8_5 emits it.
  arena_plan.report_self_hash    is immutable after Stage9 emits it.
  arena.cert.report_self_hash    is immutable after Stage9 emits it.

I-Pipeline-OP-6:
  Every emitted report and certificate must satisfy
    SelfHash(report) = report.report_self_hash.

I-Pipeline-OP-7:
  Stage9's plan does not change shape between two consecutive
  regenerations on the same upstream product hashes.

I-Pipeline-OP-8 (Reservation handshake):
  Stage9.input.overlay_plan_self_hash =
    Stage8_5.output.overlay_plan_self_hash

  ∀ r ∈ Stage8_5.output.regions.
    ∃! a ∈ Stage9.output.wram_arenas with a.named = WramOverlayRegion(r.id)
                                        ∧ a.size_bytes = r.bytes
```

## 7. Report envelope (inherited)

Both `overlay_plan.json` and `arena_plan.json` use the
`ReportEnvelope<R>` shape from F-B2/F-B4 §4 unchanged:

```rust
pub struct ReportEnvelope<R> {
    pub schema: ReportSchemaId,
    pub schema_version: SemVer,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,
    pub body: R,
}
```

Public JSON shape, envelope laws (`R-Hash`, `R-Outcome-Pass`,
`R-Outcome-Fail`, `R-FlatEnvelope`, `R-UnknownReject`,
`R-HardOnly-ThisChunk`) are inherited unchanged. Specifically:
F-B11/F-B12 reports reject `Soft` diagnostics in this chunk.

`R-NoPartialProduct` is restated for the plan products:

```text
R-NoPartialPlan-OP:
  Failed overlay_plan report
  ⇒ body.result = None

R-NoPartialPlan-AP:
  Failed arena_plan report
  ⇒ body.result = None
```

Certificates use the same envelope:

```text
arena.cert.json   uses ReportEnvelope<ArenaCertBody>
overlay.cert.json uses ReportEnvelope<OverlayCertBody>  (optional)
```

Certificates are emitted only when the corresponding plan succeeded. A
failed plan emits no certificate.

## 8. Stage 8.5 contract: `OverlayPlan`

### 8.1 Pinned inputs

```text
OverlayPlanInputs := {
    storage_plan:        StoragePlanProduct,
    sram_page_plan:      SramPagePlanProduct,
    rom_window_plan:     RomWindowPlanProduct,
    resolved_policy:     ResolvedPolicyProduct,
    runtime_chrome:      RuntimeChromeBudget,
    target_profile:      TargetProfileSummary,    -- WRAM region map
}
```

Every input is hash-bound. The pure core never reads the filesystem; it
never opens an artifact. The driver `run_stage8_5` re-validates the
input self-hashes against the upstream report envelopes before invoking
the core; an upstream-hash mismatch is a `OverlayInputHashMismatch`
diagnostic and produces a failed report with `result = None`.

The minimal projection of `ResolvedPolicyProduct` consumed by the core
is `OverlayPlanPolicyProjection`:

```text
OverlayPlanPolicyProjection := {
    overlay_eviction_default:   EvictionPolicy,
    overlay_install_event_default: OverlayInstallEvent,
    runtime_modes_requested:    NonEmptySet<RuntimeMode>,  -- audit only
}
```

K11 (§13) keys off the projection hash, not the full policy hash.

### 8.2 Pure core signature

```text
build_overlay_plan_core :
  OverlayPlanInputs -> Result<(OverlayPlan, OverlayPlanReportBody),
                              PassDiagnostics>
```

`build_overlay_plan_core` is a total function in the closed input domain.
For inputs satisfying the §8.1 hash-bound preconditions, it either
produces an `OverlayPlan` and a passing report body, or it produces a
non-empty `PassDiagnostics` whose `severity` is `Hard` for at least one
entry. `R-HardOnly-ThisChunk` (inherited from F-B2/F-B4) forbids `Soft`
diagnostics in v1.

### 8.3 Region-binding subroutine

```text
bind_regions(rwp, rcb, target_profile)
  -> Result<NonEmptyList<OverlayRegion>, PassDiagnostics>

  candidates := { k ∈ rwp.kernels | k.residency = WramOverlay }
              ∪ { l ∈ rwp.luts    | l.residency = WramOverlay }
              ∪ { e ∈ rwp.expert_fragments | e.residency = WramOverlay }

  if candidates.is_empty():
    return OverlayNoCandidatesButReservationDeclared if
           rcb.wram_overlay_cap_bytes > 0 ∧ policy demands explicit zero
    else  emit a single zero-byte region placeholder is forbidden;
           in v1, emit one region whose bytes = 0 only when explicitly
           required by RuntimeChromeBudget. Otherwise produce empty
           regions list and short-circuit to OverlayPlanProduct with no
           regions.

  partition candidates into `share_groups` keyed by:
      (region_constraint, payload_byte_size_class)
    where `region_constraint` is dictated by target_profile.wram_layout
    (e.g. WRAM bank window allowed for overlay code/data) and
    `payload_byte_size_class` clusters members whose worst-case payload
    sizes fit one shared region budget.

  For each share_group s:
    region_bytes := max over m ∈ s of m.payload_bytes
    region_bytes <= rcb.wram_overlay_region_max_bytes
                  -- enforced by RuntimeChromeBudget per-region cap;
                  -- exceeded ⇒ OverlayRegionPayloadExceedsRegionCap

    region := OverlayRegion {
      id: OverlayId(deterministic, region_index_in_canonical_order),
      bytes: region_bytes,
      constraint: target_profile.wram_layout.overlay_region_constraint,
      members: NonEmptyList::from(s.members canonical-sorted),
      reservation_kind: ReservationKind::WramOverlay,
      reservation_floor_bytes: region_bytes,
      reservation_ceil_bytes:  region_bytes,
    }

  emit ordered regions in canonical sort:
    primary: ascending region_constraint discriminant
    secondary: descending region_bytes
    tertiary: ascending min(member.id) over region.members

  Σ over r in regions of r.bytes <= rcb.wram_overlay_cap_bytes
    else  OverlayWramOverlayCapExceeded.
```

`OverlayId`s are minted by canonical sort key, not by candidate iteration
order. Two builds with identical inputs produce identical `OverlayId`s.

### 8.4 Share-class assembly

```text
assemble_share_classes(regions)
  -> SortedVec<OverlayShareClass>

For each region r whose r.members.len() >= 2:
  class := OverlayShareClass {
    id: ShareClassId(r.id, share_index = 0),
    region: r.id,
    members: r.members,
    eviction: policy.overlay_eviction_default,
  }
  yield class.

Regions with exactly one member emit no share class.
```

v1 emits at most one share class per region (§2.17). Multi-share-class
regions are forward-compatible: the schema admits them, but the v1 build
does not produce them.

The eviction policy is read from the `OverlayPlanPolicyProjection`. A
share class with `members.len() >= 2` whose `eviction` is `Undefined`
fails with `OverlayShareClassEvictionUndefined`.

### 8.5 Install scheduling

```text
schedule_installs(regions, share_classes, rwp, policy_projection)
  -> Result<SortedVec<OverlayInstall>, PassDiagnostics>

For each region r:
  For each member m in r.members:
    install := OverlayInstall {
      id: InstallId(deterministic, region.id × member.id),
      region: r.id,
      member: m,
      source: m.overlay_source,            -- per RomWindowPlan
      install_event: policy_projection.overlay_install_event_default
                     unless m.required_install_event is set,
      lease_shape: derive_lease_shape(m, target_profile),
    }
  yield install.

derive_lease_shape(m, target_profile) :=
  let source_bank := m.overlay_source.rom_bank
  in  OverlayLeaseShape {
        rom_bank_lease: RomBankLease { bank: source_bank,
                                       acquire_at: install_event.start,
                                       release_at: install_event.end },
        wram_region_lease: WramRegionLease { region: r.id,
                                             acquire_at: install_event.start,
                                             release_at: end-of-share-class-tenure },
      }

Canonical sort key for installs:
  primary:   ascending region.id
  secondary: ascending member.id
  tertiary:  ascending install_event discriminant
  quaternary: ascending source provenance

If any install's source bank is not visible at `install_event.start`
(per `RomWindowPlan`'s simultaneously-visible-set), reject with
`OverlayInstallSourceNotVisible`.
```

`OverlayLeaseShape` is a static descriptor. F-B13 mints concrete
`LeaseId`s when slices acquire and release.

### 8.6 Reservation accounting

```text
compute_reservation(regions) -> OverlayReservation:
  total_bytes := Σ over r in regions of r.bytes
  per_region  := { r.id -> ReservationEntry { bytes: r.bytes,
                                              reservation_kind: WramOverlay } }
  OverlayReservation { total_bytes, per_region, cap_bytes: rcb.wram_overlay_cap_bytes }

Hard rejects in compute_reservation:
  total_bytes > rcb.wram_overlay_cap_bytes
    ⇒ OverlayWramOverlayCapExceeded
  any r.bytes  > rcb.wram_overlay_region_max_bytes
    ⇒ OverlayRegionPayloadExceedsRegionCap
  any r.bytes  = 0 ∧ r.members.is_empty() = false
    ⇒ OverlayRegionEmptyButPopulated
  duplicate region.id
    ⇒ OverlayRegionIdDuplicate
```

The reservation is **counted, not addressed**. `OverlayPlan` does not
choose WRAM byte ranges. F-B12's arena allocator binds the addresses.

### 8.7 Self-consistency (OP-SC)

```text
OP-SC-1  Region member non-empty:
   ∀ r ∈ regions. r.members.is_empty() = false

OP-SC-2  Region member size fits region:
   ∀ r ∈ regions, ∀ m ∈ r.members.
     m.payload_bytes <= r.bytes

OP-SC-3  Region constraint admissible:
   ∀ r ∈ regions.
     r.constraint ∈ target_profile.wram_layout.allowed_overlay_constraints

OP-SC-4  Share-class membership consistent:
   ∀ s ∈ share_classes, ∀ m ∈ s.members.
     m ∈ regions[s.region].members

OP-SC-5  Eviction policy required when shared:
   ∀ s ∈ share_classes with s.members.len() >= 2.
     s.eviction is not Undefined

OP-SC-6  Install references existing region:
   ∀ i ∈ installs.
     i.region ∈ regions.ids

OP-SC-7  Install member is region member:
   ∀ i ∈ installs.
     i.member ∈ regions[i.region].members

OP-SC-8  Install source visible:
   ∀ i ∈ installs.
     i.source.rom_bank ∈ rwp.simultaneously_visible_at(i.install_event.start)

OP-SC-9  Coverage: every WramOverlay candidate is installed:
   ∀ c ∈ rwp.candidates_with_residency(WramOverlay).
     ∃ i ∈ installs with i.member = c.id

OP-SC-10 No orphan members:
   ∀ r ∈ regions, ∀ m ∈ r.members.
     ∃ i ∈ installs with i.member = m

OP-SC-11 Reservation total honors cap:
   reservation.total_bytes <= rcb.wram_overlay_cap_bytes

OP-SC-12 Reservation per-region honors region cap:
   ∀ r ∈ regions.
     reservation.per_region[r.id].bytes <= rcb.wram_overlay_region_max_bytes

OP-SC-13 Reservation entry equals region size:
   ∀ r ∈ regions.
     reservation.per_region[r.id].bytes = r.bytes

OP-SC-14 Lease shape complete:
   ∀ i ∈ installs.
     i.lease_shape.rom_bank_lease ≠ ∅
     ∧ i.lease_shape.wram_region_lease.region = i.region

OP-SC-15 Canonical sort hash stable:
   regions, share_classes, installs are pre-sorted by their canonical
   keys before self-hash; resort and rehash must yield identical bytes.

OP-SC-16 No section / no codegen leakage:
   No field carries SectionRole, AsmIR, BankPlacement, or any byte-level
   payload. `OverlayInstall.source` is provenance, not bytes.

OP-SC-17 No scheduling fields:
   No SliceId, LeaseId, ResourceVector, CycleBudget, YieldKind,
   ExitKind appears in OverlayPlan. `OverlayInstallEvent` is a static
   descriptor; F-B13 mints slice ids.

OP-SC-18 No repair provenance:
   ∀ d ∈ overlay_plan.report.diagnostics.
     d.provenance.policy_source ⊆ {TargetDefault, ProfileDefault,
                                   CompileRequestOverride, HintBundle,
                                   Calibration}.
```

Each `OP-SC-k` corresponds to at least one fixture under
`fixtures/overlay_plan/reject/`.

### 8.8 OverlayPlan identity and self-hash

```text
overlay_plan.identity := OverlayPlanIdentity {
    storage_plan_self_hash:     sp.self_hash,
    sram_page_plan_self_hash:   spp.self_hash,
    rom_window_plan_self_hash:  rwp.self_hash,
    policy_projection_hash:     DomainHash("gbf-codegen",
                                            "OverlayPlanPolicyProjection",
                                            "v1",
                                            CanonicalJson(projection)),
    runtime_chrome_budget_hash: rcb.self_hash,
    target_profile_hash:        target_profile.self_hash,
    pass_version:               "stage8_5/v1",
    crate_feature_set_hash:     DomainHash("gbf-codegen", "FeatureSet",
                                            "v1", canonical_feature_set_bytes),
}

overlay_plan_self_hash := DomainHash(
    "gbf-codegen", "OverlayPlan", "overlay_plan.v1",
    CanonicalJson(OverlayPlan_with_identity))
```

Auditor parents (`policy_resolution_self_hash`, `compile_request_hash`)
are recorded in `OverlayPlanReportBody.input_identity` for audit only;
they never invalidate K11.

## 9. Stage 9 contract: `ArenaPlan`

### 9.1 Pinned inputs

```text
ArenaPlanInputs := {
    storage_plan:        StoragePlanProduct,
    sram_page_plan:      SramPagePlanProduct,
    rom_window_plan:     RomWindowPlanProduct,
    overlay_plan:        OverlayPlanProduct,
    resolved_policy:     ResolvedPolicyProduct,
    runtime_chrome:      RuntimeChromeBudget,
    target_profile:      TargetProfileSummary,
}
```

Every input is hash-bound. The driver re-validates self-hashes; any
mismatch is `ArenaInputHashMismatch`. The reservation handshake
(`I-Pipeline-OP-8`) requires
`ArenaPlanInputs.overlay_plan.self_hash = OverlayPlanReady.product.self_hash`.

`ArenaPlanPolicyProjection`:

```text
ArenaPlanPolicyProjection := {
    arena_alignment_default:    ArenaAlignment,
    arena_zerofill_policy:      ZerofillPolicy,
    persistent_page_geometry:   PersistentPageGeometry,
}
```

K12 keys off the projection hash, not the full policy hash.

### 9.2 Pure core signature

```text
build_arena_plan_core :
  ArenaPlanInputs -> Result<(ArenaPlan, ArenaPlanReportBody),
                            PassDiagnostics>
```

Total in the closed input domain. For valid inputs, returns `(plan, body)`
or non-empty `Hard` `PassDiagnostics`.

### 9.3 ArenaPlan shape

```text
ArenaPlan := {
    identity:              ArenaPlanIdentity,
    wram_arenas:           SortedVec<ArenaInstance>,
    sram_arenas:           SortedVec<ArenaInstance>,
    hram_assignments:      SortedVec<ArenaInstance>,
    overlay_reservation:   OverlayReservationHonor,
    arena_bindings:        ArenaBindings,
}

ArenaInstance := {
    id:           ArenaId,
    named:        NamedArena,
    byte_range:   ByteRange { start: u16, len: u16 },
    backing:      MemoryBacking,           -- WRAM | SRAM | HRAM
    alignment:    ArenaAlignment,
    zerofill:     ZerofillPolicy,
    slots:        SortedVec<ArenaSlot>,
}

ArenaSlot := {
    id:                 ArenaSlotId,
    byte_offset:        u16,                -- relative to arena start
    size_bytes:         u16,
    alias_class_id:     AliasClassId,
    lifetime_class:     LifetimeClass,
    binding_kind:       SlotBindingKind,
    binding_ref:        SlotBindingRef,
}

SlotBindingKind :=
   MaterializedValue
 | PersistentPageA(PersistPageId, CommitGroupId)
 | PersistentPageB(PersistPageId, CommitGroupId)
 | OverlayMember(OverlayId, OverlayResidentId)
 | RuntimeFixed(RuntimeFixedKind)             -- continuation, harness, etc.
 | TraceRing(TraceRingId)

SlotBindingRef :=
   ValueId(ValueId)                            -- for MaterializedValue
 | PersistPageRef { page: PersistPageId,
                    commit_group: CommitGroupId }
 | OverlayResidentRef(OverlayResidentId)
 | RuntimeFixedRef(RuntimeFixedKind)
 | TraceRingRef(TraceRingId)

ArenaBindings := {
    materialize_to_slot:   Map<StorageBindingId, ArenaSlotId>,
    persist_to_slot_pair:  Map<(PersistPageId, CommitGroupId),
                                (ArenaSlotId, ArenaSlotId)>,    -- A and B
    overlay_to_arena:      Map<OverlayId, ArenaId>,
}

OverlayReservationHonor := {
    total_bytes:           u16,
    per_region:            SortedVec<ReservationHonorEntry>,
}

ReservationHonorEntry := {
    overlay_id:    OverlayId,
    arena_id:      ArenaId,
    bytes:         u16,                -- equals OverlayPlan.regions[i].bytes
    byte_range:    ByteRange,
}
```

`ArenaSlotId` is allocated by canonical sort over slot bindings; it does
not depend on the FFD iteration order.

### 9.4 Allocation algorithm (FFD)

```text
For each MemoryBacking ∈ { WRAM, SRAM, HRAM }:
  Collect all ArenaInstances in that backing.
  For each ArenaInstance a:
    free := [ FreeInterval { start: 0, len: a.byte_range.len } ]

    slots_to_place := canonical-sort(a.slots) by:
      primary:    descending size_bytes
      secondary:  ascending alias_class_id
      tertiary:   ascending lifetime_class_priority
      quaternary: ascending binding_kind discriminant
      quinary:    ascending binding_ref provenance

    for slot in slots_to_place:
      pick first interval iv in free with
        iv.len >= round_up(slot.size_bytes, a.alignment)
      if no such iv:  ArenaAllocationFailed { arena: a.id, slot: slot.id }
      align := round_up_offset(iv.start, a.alignment)
      slot.byte_offset := align
      shrink iv from [align, align + slot.size_bytes) within free.
      if slot.alias_class_id is in MustOverlap-set with another slot s'
        already placed in a:
          assert s'.byte_offset = slot.byte_offset
                  ∧ s'.size_bytes = slot.size_bytes
        else  AliasClassMustOverlapDisagreement.

    Post-allocation:
      Σ over slot ∈ a.slots of round_up(slot.size_bytes, a.alignment)
        <= a.byte_range.len
      else  ArenaCapacityExceeded.
```

Determinism axioms for the algorithm:

```text
F-FFD-Det-1:
  For fixed (slots, alignment, arena.byte_range.len), the FFD output
  byte_offsets are uniquely determined by the canonical sort key.

F-FFD-Det-2:
  Free-interval bookkeeping uses a deterministic data structure
  (sorted vector by start) and never relies on hash-based iteration.
```

### 9.5 Arena selection from LifetimeClass and StorageClass

```text
arena_for(StorageBinding b) :=
  match b.materialization:
    Materialize { class: StorageClass::TokenScratch,    lifetime: Slice }
      -> WramAccumScratch | WramRouteScratch | WramDecodeScratch
         (per StorageBinding.scratch_role)
    Materialize { class: StorageClass::Activation,      lifetime: Token }
      -> WramActivationsPingA | WramActivationsPingB
         (selected by ping_index from StoragePlan)
    Materialize { class: StorageClass::Activation,      lifetime: ResumeWindow }
      -> WramActivationsPingA | WramActivationsPingB
    Materialize { class: StorageClass::WramHot, lifetime: Session }
      -> WramContinuationRecord
    Materialize { class: StorageClass::PersistedTranscript,
                  lifetime: Session }
      -> SramPersistedTranscript
    Materialize { class: StorageClass::ColdSpill,       lifetime: ResumeWindow }
      -> SramColdSpill
    Materialize { class: StorageClass::Trace,           lifetime: Session }
      -> SramTracePages
    Persist { page, commit_group }
      -> SramSequenceStatePages(stream_id_of(page))
    other  ⇒ ArenaUnmappedStorageClass

arena_for runtime-fixed assignments:
  RuntimeFixedKind::HarnessCommandBlock -> SramHarnessCommandBlock
  RuntimeFixedKind::HarnessResultBlock  -> SramHarnessResultBlock
  RuntimeFixedKind::FrameFlags          -> HramFrameFlags
  RuntimeFixedKind::BankShadow          -> HramBankShadow
  RuntimeFixedKind::FaultCode           -> HramFaultCode
  RuntimeFixedKind::SchedulerScratch    -> HramSchedulerScratch
  RuntimeFixedKind::YieldRequested      -> HramYieldRequested
```

The `arena_for` map is closed in v1. `ArenaUnmappedStorageClass` is a
hard reject.

### 9.6 Persistent-page geometry binding

```text
For each (page, commit_group) ∈ sram_page_plan.persistent_pages:
  geom := arena_policy.persistent_page_geometry
  page_size := sizeof(PersistHeader) + payload_bytes_of(page)
                  + sizeof_commit_word(geom)
  alignment := geom.page_alignment

  Allocate two slots in arena `SramSequenceStatePages(stream_id_of(page))`:
    slot_a := PersistentPageA(page, commit_group), size = page_size
    slot_b := PersistentPageB(page, commit_group), size = page_size

  Both slots share alias_class = AliasClassId::pair(page, commit_group)
  but are NOT in MustOverlap relation: byte_offsets differ.

  Within each page, internal layout follows PersistHeader + payload +
  commit_word geometry per §2.7.

  Sequence-state arenas for one stream_id contain pages of the same
  stream's commit groups; cross-stream sharing is forbidden
  (ArenaCrossStreamPageSharing).
```

### 9.7 Reservation honoring subroutine

```text
honor_overlay_reservation(op, wram_arenas) -> Result<OverlayReservationHonor, ...>

For each region r ∈ op.regions:
  build a WramOverlayRegion(r.id) arena:
    arena := ArenaInstance {
      id: ArenaId(r.id),
      named: NamedArena::WramOverlayRegion(r.id),
      byte_range: allocate_in_wram(size = r.bytes,
                                   constraint = r.constraint),
      backing: MemoryBacking::WRAM,
      alignment: ArenaAlignment::Word,    -- per RuntimeChromeBudget
      zerofill: arena_policy.zerofill_default,
      slots: NonEmptyList::from(
                r.members.map(m -> ArenaSlot {
                  id: deterministic,
                  byte_offset: 0,
                  size_bytes: m.payload_bytes,
                  alias_class_id: AliasClassId::overlay(r.id, m.id),
                  lifetime_class: LifetimeClass::OverlayMember,
                  binding_kind: OverlayMember(r.id, m.id),
                  binding_ref: OverlayResidentRef(m.id),
                })),
    }

  Members of the same region share byte_range exactly: each slot's
  byte_offset is 0 within the region, since installs time-share the
  region. Two members of the same share class must have IDENTICAL
  byte_range start (offset 0) and size_bytes <= region.bytes.

  Hard checks:
    arena.byte_range.len = r.bytes      else  ArenaOverlayReservationOverflow
                                              | ArenaOverlayReservationUnderflow
    arena.byte_range disjoint from non-overlay WRAM arenas
                                       else  ArenaOverlayReservationOverlap
```

The "two members share offset 0" rule is what makes share-class
time-multiplexing work: the runtime overwrites the same WRAM bytes when
it installs a different member, so all members must start at the same
offset. Different sizes are allowed; the *region size* is the maximum
member size (§8.3).

### 9.8 Self-consistency (AP-SC)

```text
AP-SC-1  Materialize coverage:
   ∀ b ∈ sp.bindings with b.materialization = Materialize{...}.
     ∃! slot ∈ arena_bindings.materialize_to_slot[b.id]

AP-SC-2  Persist coverage:
   ∀ b ∈ sp.bindings with b.materialization = Persist{page, commit_group}.
     ∃! (slot_a, slot_b) ∈ arena_bindings.persist_to_slot_pair
                                            [(page, commit_group)]

AP-SC-3  Recompute non-allocation:
   ∀ b ∈ sp.bindings with b.materialization = Recompute.
     b.id ∉ arena_bindings.materialize_to_slot.keys

AP-SC-4  Pure expression non-allocation:
   ∀ value v with v.role = PureExpression and v.materialization = Recompute.
     no ArenaSlot binds v.

AP-SC-5  AliasClass equality on shared slot:
   ∀ slot s1, s2 with s1.byte_range = s2.byte_range
                  ∧ s1.arena_id = s2.arena_id.
     s1.alias_class_id = s2.alias_class_id.

AP-SC-6  AliasClass inequality enforces disjointness:
   ∀ slot s1, s2 with s1.alias_class_id ≠ s2.alias_class_id
                  ∧ s1.arena_id = s2.arena_id.
     s1.byte_range ∩ s2.byte_range = ∅.

AP-SC-7  LifetimeClass preserved:
   ∀ slot s ∈ arena_bindings.materialize_to_slot.values.
     s.lifetime_class = sp.bindings[binding_id_of(s)].materialization.lifetime

AP-SC-8  LifetimeClass admissible for arena:
   ∀ slot s in arena a.
     a.named ∈ allowed_arena_family(s.lifetime_class, s.binding_kind)

AP-SC-9  Persistent-page geometry valid:
   ∀ (page, cg, slot_a, slot_b) ∈ persist_to_slot_pair.
     slot_a.size_bytes = slot_b.size_bytes = sizeof(PersistHeader)
        + payload_bytes(page) + sizeof_commit_word(geom)
     slot_a.byte_offset ≠ slot_b.byte_offset
     both slots reside in arena
        SramSequenceStatePages(stream_id_of(page)).

AP-SC-10 Overlay reservation honored:
   overlay_reservation.total_bytes = op.reservation.total_bytes
   ∀ r ∈ op.regions.
     overlay_reservation.per_region[r.id].bytes = r.bytes

AP-SC-11 Overlay arena disjoint:
   ∀ r ∈ op.regions, ∀ a ∈ wram_arenas with a.named ≠ WramOverlayRegion(r.id).
     wram_arenas.lookup(WramOverlayRegion(r.id)).byte_range ∩
        a.byte_range = ∅

AP-SC-12 SRAM no-span:
   ∀ s ∈ sram_arenas.
     s.byte_range fits entirely within one 8 KiB SRAM bank window.

AP-SC-13 Harness arena no-leak:
   ∀ slot s with s.binding_kind = RuntimeFixed(HarnessCommandBlock |
                                               HarnessResultBlock).
     s.arena_id refers to SramHarnessCommandBlock or
                          SramHarnessResultBlock,
     and disjoint from every SramSequenceStatePages, SramPersistedTranscript,
     SramColdSpill, SramTracePages slot.

AP-SC-14 HRAM single-byte alignment:
   ∀ a ∈ hram_assignments.
     a.alignment = ArenaAlignment::Byte.
     a.byte_range fits within HRAM
        [target_profile.hram_base, target_profile.hram_base + 0x80).

AP-SC-15 Continuation record matches abi:
   ∃! a ∈ wram_arenas with a.named = WramContinuationRecord.
   a.byte_range.len =
       sizeof_repr_c(InferenceStateHeader) + continuation_tail_window_bytes.
   a.slots[header_slot].byte_offset = 0.
   a.slots[header_slot].size_bytes = sizeof_repr_c(InferenceStateHeader).
   arena.cert.v1.continuation_record.tail_slots is sorted by byte_offset
       then ArenaSlotId, resolves every WramHot/Session materialized
       ValueId, and its maximum tail slot end equals a.byte_range.len.

AP-SC-16 Bank0 WRAM/HRAM caps:
   wram_arenas.bank0_wram_bytes <=
       rcb.wram_runtime_floor_bytes
   hram_assignments.total_bytes <= rcb.hram_usable_bytes

AP-SC-17 No section / no codegen leakage:
   No ArenaInstance carries a SectionRole, BankPlacement, AsmIR, or any
   byte-level payload. byte_range is an offset/length only.

AP-SC-18 No scheduling fields:
   No SliceId, LeaseId, ResourceVector, CycleBudget appears in ArenaPlan.

AP-SC-19 No repair provenance:
   ∀ d ∈ arena_plan.report.diagnostics.
     d.provenance.policy_source ⊆ {TargetDefault, ProfileDefault,
                                   CompileRequestOverride, HintBundle,
                                   Calibration}.

AP-SC-20 Canonical sort hash stable:
   wram_arenas, sram_arenas, hram_assignments, slots, and arena_bindings
   are pre-sorted by canonical keys before self-hash.

AP-SC-21 Pure expression no-slot:
   No ArenaSlot's binding_kind references a ValueId whose StoragePlan
   binding is Recompute.

AP-SC-22 Cross-arena slot uniqueness:
   ArenaSlotIds are unique across all arenas.

AP-SC-23 Trace ring placement:
   ∀ slot s ∈ SramTracePages with s.binding_kind = TraceRing(_).
     s lives in SramTracePages and not elsewhere.

AP-SC-24 Reservation entry size equality:
   ∀ r ∈ op.regions.
     overlay_reservation.per_region[r.id].bytes = r.bytes.

AP-SC-25 Reservation honor count:
   |overlay_reservation.per_region| = |op.regions|.
```

### 9.9 ArenaPlan identity and self-hash

```text
arena_plan.identity := ArenaPlanIdentity {
    storage_plan_self_hash:     sp.self_hash,
    sram_page_plan_self_hash:   spp.self_hash,
    rom_window_plan_self_hash:  rwp.self_hash,
    overlay_plan_self_hash:     op.self_hash,
    policy_projection_hash:     DomainHash("gbf-codegen",
                                            "ArenaPlanPolicyProjection",
                                            "v1",
                                            CanonicalJson(projection)),
    runtime_chrome_budget_hash: rcb.self_hash,
    target_profile_hash:        target_profile.self_hash,
    pass_version:               "stage9/v1",
    crate_feature_set_hash:     DomainHash("gbf-codegen", "FeatureSet",
                                            "v1", canonical_feature_set_bytes),
}

arena_plan_self_hash := DomainHash(
    "gbf-codegen", "ArenaPlan", "arena_plan.v1",
    CanonicalJson(ArenaPlan_with_identity))
```

Auditor parents (`policy_resolution_self_hash`, `compile_request_hash`)
are recorded in `ArenaPlanReportBody.input_identity`; they never
invalidate K12.

## 10. Address invariants

This section pins the global address-space invariants that hold when both
plans pass. They are restatable as predicates over the joint product
`(OverlayPlan, ArenaPlan)`.

```text
F-Addr-1 (every materialized value has exactly one slot):
  ∀ b ∈ sp.bindings with b.materialization = Materialize{...}.
    | { slot ∈ ap.slots(*) | slot.binding_ref = ValueId(b.value) } | = 1

F-Addr-2 (every persisted (page, commit_group) maps to a slot pair):
  ∀ b ∈ sp.bindings with b.materialization = Persist{page, cg}.
    ∃! pair ∈ ap.arena_bindings.persist_to_slot_pair[(page, cg)]
       with pair.0.binding_kind = PersistentPageA(page, cg)
        ∧  pair.1.binding_kind = PersistentPageB(page, cg)
        ∧  pair.0.size_bytes = pair.1.size_bytes

F-Addr-3 (alias-class slot sharing is total or disjoint):
  ∀ slot s1, s2 ∈ ap.slots(*) with s1.arena_id = s2.arena_id.
    s1.byte_range overlaps s2.byte_range
      ⇒ (s1 = s2)
        ∨ (s1.alias_class_id = s2.alias_class_id
           ∧ s1.byte_range = s2.byte_range)

F-Addr-4 (lifetime preservation):
  ∀ slot s ∈ ap.materialize_to_slot.values.
    s.lifetime_class = sp.bindings[binding_id_of(s)].materialization.lifetime

F-Addr-5 (overlay region byte equality):
  ∀ r ∈ op.regions.
    let arena := ap.wram_arenas.lookup(WramOverlayRegion(r.id))
    arena.byte_range.len = r.bytes

F-Addr-6 (overlay disjointness):
  ∀ r ∈ op.regions, ∀ a ∈ ap.wram_arenas with a.named ≠ WramOverlayRegion(r.id).
    arena_for(WramOverlayRegion(r.id)).byte_range ∩ a.byte_range = ∅

F-Addr-7 (overlay member start-of-region):
  ∀ r ∈ op.regions, ∀ m ∈ r.members.
    let arena := ap.wram_arenas.lookup(WramOverlayRegion(r.id))
    let slot  := arena.slots.find(binding_kind = OverlayMember(r.id, m.id))
    slot.byte_offset = 0
    slot.size_bytes <= r.bytes

F-Addr-8 (no SRAM span):
  ∀ s ∈ ap.sram_arenas.
    s.byte_range fits in one 8 KiB SRAM bank window.

F-Addr-9 (HRAM page bound):
  ∀ a ∈ ap.hram_assignments.
    a.byte_range ⊆ [target_profile.hram_base,
                    target_profile.hram_base + 0x80)

F-Addr-10 (continuation record sized):
  ap.wram_arenas.lookup(WramContinuationRecord).byte_range.len
    = sizeof_repr_c(InferenceStateHeader) + continuation_tail_window_bytes

F-Addr-11 (harness arena disjoint from data arenas):
  ap.sram_arenas.lookup(SramHarnessCommandBlock).byte_range
    ∩ ap.sram_arenas.lookup(SramSequenceStatePages(_)).byte_range = ∅
  ap.sram_arenas.lookup(SramHarnessResultBlock).byte_range
    ∩ ap.sram_arenas.lookup(SramSequenceStatePages(_)).byte_range = ∅
  similarly for SramPersistedTranscript, SramColdSpill, SramTracePages.

F-Addr-12 (persistent-page header alignment):
  ∀ slot s ∈ ap.persist pages.
    s.byte_offset is aligned per geom.page_alignment.
    The first sizeof(PersistHeader) bytes of s are reserved for the
    PersistHeader; the last sizeof_commit_word bytes for the commit word.

F-Addr-13 (slot id uniqueness):
  ArenaSlotIds are unique across ap.

F-Addr-14 (every ArenaSlot binding_ref resolves):
  ∀ slot s ∈ ap.slots(*).
    s.binding_ref points to an entity whose existence is asserted by
    one of: sp.bindings, op.regions/installs, runtime fixed table,
    sram_page_plan.persistent_pages, target_profile.trace_rings.
```

`F-Addr-1..14` are the address invariants F-B13 may rely on without
re-proving them.

## 11. Reservation accounting

The reservation handshake between F-B11 and F-B12 is reified as a typed
record so that downstream consumers (F-B13, F-B14, F-C3) can audit it
without re-deriving it.

### 11.1 OverlayReservation

```text
OverlayReservation := {
    total_bytes:           u16,
    per_region:            SortedVec<ReservationEntry>,
    cap_bytes:             u16,                -- rcb.wram_overlay_cap_bytes
    region_max_bytes:      u16,                -- rcb.wram_overlay_region_max_bytes
}

ReservationEntry := {
    overlay_id:    OverlayId,
    bytes:         u16,
    reservation_kind: ReservationKind::WramOverlay,
}
```

`OverlayReservation` is part of `OverlayPlan` and lives at
`OverlayPlan.reservation`. It is NOT a sidecar.

### 11.2 OverlayReservationHonor

```text
OverlayReservationHonor := {
    total_bytes:    u16,
    per_region:     SortedVec<ReservationHonorEntry>,
}

ReservationHonorEntry := {
    overlay_id:  OverlayId,
    arena_id:    ArenaId,                    -- the arena bound to this region
    bytes:       u16,                         -- equals region.bytes
    byte_range:  ByteRange,                   -- WRAM offset/length
}
```

`OverlayReservationHonor` is part of `ArenaPlan` and lives at
`ArenaPlan.overlay_reservation`. It is NOT a sidecar.

### 11.3 Honor predicate

```text
HonorPredicate(op, ap) :=
    ap.overlay_reservation.total_bytes = op.reservation.total_bytes
  ∧ |ap.overlay_reservation.per_region| = |op.reservation.per_region|
  ∧ ∀ r ∈ op.reservation.per_region.
       ∃! e ∈ ap.overlay_reservation.per_region with
          e.overlay_id = r.overlay_id
        ∧ e.bytes      = r.bytes
        ∧ e.byte_range.len = r.bytes
```

Failure modes:

```text
ArenaOverlayReservationOverflow:
  ∃ r. ap.lookup(r).bytes > op.regions[r].bytes
    Hard reject.

ArenaOverlayReservationUnderflow:
  ∃ r. ap.lookup(r).bytes < op.regions[r].bytes
    Hard reject.

ArenaOverlayReservationOverlap:
  ∃ r, a ∈ ap.wram_arenas, a.named ≠ WramOverlayRegion(r.id).
    ap.lookup(WramOverlayRegion(r.id)).byte_range ∩ a.byte_range ≠ ∅
    Hard reject.

ArenaOverlayReservationCountMismatch:
  |ap.overlay_reservation.per_region| ≠ |op.reservation.per_region|
    Hard reject.

OverlayReservationCapDrift:
  op.reservation.cap_bytes ≠ rcb.wram_overlay_cap_bytes
    Hard reject in OverlayPlan construction (caught by §8.6).
```

### 11.4 Reservation in StageCache identities

```text
ArenaPlanIdentity.overlay_plan_self_hash binds the OverlayPlan exactly.
A reservation drift between the inputs presented to the core and the
inputs the core actually consumes is impossible by construction:
the reservation is part of OverlayPlan, and OverlayPlan's self-hash
covers it.
```

## 12. Report schemas

### 12.1 `overlay_plan.v1`

```text
overlay_plan.v1.body := OverlayPlanReportBody {
    schema_id:           "overlay_plan.v1",
    pass_version:        "stage8_5/v1",
    input_identity:      OverlayPlanInputIdentityRecord,
    audit_parents:       OverlayPlanAuditParents,
    diagnostics:         Vec<ValidationDiagnostic>,
    result:              Option<OverlayPlanResult>,
}

OverlayPlanInputIdentityRecord := {
    storage_plan_self_hash:     Hash256,
    sram_page_plan_self_hash:   Hash256,
    rom_window_plan_self_hash:  Hash256,
    runtime_chrome_budget_hash: Hash256,
    target_profile_hash:        Hash256,
    overlay_plan_policy_projection_hash: Hash256,
}

OverlayPlanAuditParents := {
    policy_resolution_self_hash:    Hash256,
    artifact_validation_self_hash:  Hash256,
    compile_request_hash:           Hash256,
}

OverlayPlanResult := {
    product:              OverlayPlan,
    overlay_plan_self_hash: Hash256,           -- equal to product.identity-derived
    summary:              OverlayPlanSummary,  -- review aid; redundant
}

OverlayPlanSummary := {
    region_count:        u16,
    share_class_count:   u16,
    install_count:       u16,
    reserved_bytes:      u16,
    cap_bytes:           u16,
}
```

`R-NoPartialPlan-OP` enforces `result = None` on failure. Diagnostics
are typed `ValidationDiagnostic`s with `origin = OverlayPlanConstruction`.

### 12.2 `arena_plan.v1`

```text
arena_plan.v1.body := ArenaPlanReportBody {
    schema_id:           "arena_plan.v1",
    pass_version:        "stage9/v1",
    input_identity:      ArenaPlanInputIdentityRecord,
    audit_parents:       ArenaPlanAuditParents,
    diagnostics:         Vec<ValidationDiagnostic>,
    result:              Option<ArenaPlanResult>,
}

ArenaPlanInputIdentityRecord := {
    storage_plan_self_hash:     Hash256,
    sram_page_plan_self_hash:   Hash256,
    rom_window_plan_self_hash:  Hash256,
    overlay_plan_self_hash:     Hash256,
    runtime_chrome_budget_hash: Hash256,
    target_profile_hash:        Hash256,
    arena_plan_policy_projection_hash: Hash256,
}

ArenaPlanAuditParents := {
    policy_resolution_self_hash:    Hash256,
    artifact_validation_self_hash:  Hash256,
    compile_request_hash:           Hash256,
}

ArenaPlanResult := {
    product:              ArenaPlan,
    arena_plan_self_hash: Hash256,
    summary:              ArenaPlanSummary,
}

ArenaPlanSummary := {
    wram_arena_count:    u16,
    sram_arena_count:    u16,
    hram_assignment_count: u16,
    materialize_slot_count: u16,
    persist_slot_pair_count: u16,
    overlay_reservation_total_bytes: u16,
    wram_total_bytes:    u16,
    sram_total_bytes:    u16,
    hram_total_bytes:    u16,
}
```

### 12.3 `arena.cert.v1`

```text
arena.cert.v1.body := ArenaCertBody {
    schema_id:                "arena.cert.v1",
    pass_version:             "stage9/v1",
    input_hashes:             ArenaPlanInputHashes, -- all non-zero, pins
                                                     -- storage/sram/window/
                                                     -- overlay/runtime/policy
    arena_plan_self_hash:     Hash256,
    address_invariants:       AddressInvariantsCertificate,
    reservation_honor:        OverlayReservationHonor,
    continuation_record:      ContinuationRecordCertificate,
    persistent_page_geometry: PersistentPageGeometryCertificate,
    harness_no_leak:          HarnessNoLeakCertificate,
}

AddressInvariantsCertificate := {
    materialize_coverage:           bool,    -- F-Addr-1
    persist_coverage:               bool,    -- F-Addr-2
    alias_class_share_or_disjoint:  bool,    -- F-Addr-3
    lifetime_preservation:          bool,    -- F-Addr-4
    overlay_byte_equality:          bool,    -- F-Addr-5
    overlay_disjointness:           bool,    -- F-Addr-6
    overlay_member_start:           bool,    -- F-Addr-7
    sram_no_span:                   bool,    -- F-Addr-8
    hram_page_bound:                bool,    -- F-Addr-9
    continuation_sized:             bool,    -- F-Addr-10
    harness_disjoint:               bool,    -- F-Addr-11
    persist_header_aligned:         bool,    -- F-Addr-12
    slot_id_unique:                 bool,    -- F-Addr-13
    binding_ref_resolves:           bool,    -- F-Addr-14
}

PersistentPageGeometryCertificate := {
    geometry:           PersistentPageGeometry,
    pages:              SortedVec<PersistentPageRecord>,
}

PersistentPageRecord := {
    page:               PersistPageId,
    commit_group:       CommitGroupId,
    stream_id:          SequenceStreamId,
    page_a_byte_range:  ByteRange,
    page_b_byte_range:  ByteRange,
    page_size_bytes:    u16,
}

ContinuationRecordCertificate := {
    abi_symbol:         "gbf_abi::InferenceStateHeader",
    arena_id:           ArenaId,
    byte_range:         ByteRange,
    header_slot:        ArenaSlotId,
    header_size_bytes:  sizeof_repr_c(InferenceStateHeader),
    tail_size_bytes:    u16,
    total_size_bytes:   header_size_bytes + tail_size_bytes,
    tail_slots:         SortedVec<ContinuationTailSlotCertificate>,
}

ContinuationTailSlotCertificate := {
    slot:         ArenaSlotId,
    value:        ValueId,
    byte_offset:  u16,
    size_bytes:   u16,
}

HarnessNoLeakCertificate := {
    command_block_arena: ArenaId,
    result_block_arena:  ArenaId,
    disjoint_from:       SortedVec<NamedArena>,
}
```

The certificate's `address_invariants` flags must be `true` for every
field; any `false` flag reduces to `result = None` in the corresponding
`arena_plan.json` (the certificate is emitted only on plan success).

### 12.4 `overlay.cert.v1` (optional, "Amends planv0")

`planv0.md` lists `arena.cert.json` but not `overlay.cert.json`. This
RFC adds an optional certificate for OverlayPlan as a forward-compatible
extension. v1 emission is gated by `compile_knobs.emit_overlay_cert`
(named-only; the knob is wired through `ResolvedCompilePolicy`):

```text
overlay.cert.v1.body := OverlayCertBody {
    schema_id:                  "overlay.cert.v1",
    pass_version:               "stage8_5/v1",
    overlay_plan_self_hash:     Hash256,
    rom_window_plan_self_hash:  Hash256,
    runtime_chrome_budget_hash: Hash256,
    reservation:                OverlayReservation,
    install_visibility:         SortedVec<InstallVisibilityRecord>,
    coverage:                   OverlayCoverageCertificate,
}

InstallVisibilityRecord := {
    install_id:        InstallId,
    region:            OverlayId,
    member:            OverlayResidentId,
    source_bank:       RomBank,
    install_event:     OverlayInstallEvent,
    visible_at_start:  bool,
}

OverlayCoverageCertificate := {
    candidates_total:  u16,
    candidates_installed: u16,
    candidates_covered: bool,                -- OP-SC-9
}
```

When the knob is disabled (default), no `overlay.cert.json` is emitted.
F-B12's `arena.cert.json` is always emitted.

Amends planv0: planv0.md §"Reports and artifacts" lists `arena.cert.json`
but not `overlay.cert.json`. This RFC adds the optional certificate.

### 12.5 Canonical JSON discipline

Both reports follow F-B2/F-B4 §2.5 unchanged:

```text
- UTF-8, lex-sorted object keys, integers only, no NaN/Inf.
- No unknown fields in deserialization (RUnknownReject).
- Explicit enum tags for every sum-type variant.
- Deterministic ordering for arrays where order is not semantically meaningful:
    regions, share_classes, installs, wram_arenas, sram_arenas,
    hram_assignments, slots, per_region, materialize_to_slot pairs,
    persist_to_slot_pair pairs.
- Null only for explicit semantic absence (Option in Rust types).
- Diagnostics carry typed `code`, `severity`, `origin`, `detail`,
  `provenance` per F-B2/F-B4 §5.
```

### 12.6 Round-trip

Both reports round-trip:

```text
parse(report_bytes)
  ⇒ canonicalize
  ⇒ semantic-validate (per OP-SC / AP-SC)
  ⇒ self-hash equality (R-Hash)
  ⇒ embedded product round-trip (OverlayPlan / ArenaPlan)
  ⇒ re-emit byte-identical canonical JSON
```

Round-trip failures are typed as `OverlayReportRoundTripFailed` /
`ArenaReportRoundTripFailed`.

## 13. StageCache algebra

### 13.1 K11 (Stage 8.5)

```text
OverlayPlanCacheKey := DomainHash(
    "gbf-codegen", "StageCacheKey", "overlay_plan", "v1",
    CanonicalJson(OverlayPlanCacheKeyBody))

OverlayPlanCacheKeyBody := {
    storage_plan_self_hash:     Hash256,
    sram_page_plan_self_hash:   Hash256,
    rom_window_plan_self_hash:  Hash256,
    runtime_chrome_budget_hash: Hash256,
    target_profile_hash:        Hash256,
    overlay_plan_policy_projection_hash: Hash256,
    pass_version:               "stage8_5/v1",
    crate_feature_set_hash:     Hash256,
}
```

`policy_resolution_self_hash` is NOT a K11 input. K11 sees only the
projection. Audit parents are recorded in `OverlayPlanReportBody.audit_parents`
and never invalidate K11.

### 13.2 K12 (Stage 9)

```text
ArenaPlanCacheKey := DomainHash(
    "gbf-codegen", "StageCacheKey", "arena_plan", "v1",
    CanonicalJson(ArenaPlanCacheKeyBody))

ArenaPlanCacheKeyBody := {
    storage_plan_self_hash:     Hash256,
    sram_page_plan_self_hash:   Hash256,
    rom_window_plan_self_hash:  Hash256,
    overlay_plan_self_hash:     Hash256,
    runtime_chrome_budget_hash: Hash256,
    target_profile_hash:        Hash256,
    arena_plan_policy_projection_hash: Hash256,
    pass_version:               "stage9/v1",
    crate_feature_set_hash:     Hash256,
}
```

### 13.3 Cache laws

```text
F-Cache-K11:
  Two builds with identical K11 hit the cache and replay byte-identical
  overlay_plan.json (and overlay.cert.json if enabled).

F-Cache-K12:
  Two builds with identical K12 hit the cache and replay byte-identical
  arena_plan.json and arena.cert.json.

F-Cache-Failure:
  Failure memos cache the typed PassDiagnostics of a Hard failure.
  A failure memo is NEVER usable as a success product.

F-Cache-Drift:
  Cache miss occurs if any of:
    pass_version, schema_id, schema_version, crate_feature_set_hash,
    or any input self-hash drifts.

F-Cache-Read-Validate:
  On cache hit, the cached report's self_hash must equal R-Hash of the
  cached body. If not, the entry is poisoned and recomputed.
```

### 13.4 K11/K12 cross-validation

```text
F-K11-K12-Pinning:
  K12.overlay_plan_self_hash must equal the OverlayPlan product hash
  produced under K11 with the same upstream inputs.

F-K11-K12-NoStaleness:
  If K11 misses (any upstream drift), K12 must also miss because
  overlay_plan_self_hash will differ.
```

## 14. Diagnostic algebra

### 14.1 New `ValidationOrigin` variants

```text
ValidationOrigin (extension) :=
  ...existing F-B2/F-B4/F-B3/F-B5/F-B6/F-B7/F-B8/F-B9/F-B10 origins...
  | OverlayPlanConstruction
  | ArenaPlanConstruction
```

Both extend the closed enum without modifying existing variants.

### 14.2 OverlayPlan diagnostic codes

```text
OverlayInputHashMismatch
OverlayWramOverlayCapExceeded
OverlayRegionPayloadExceedsRegionCap
OverlayRegionEmptyButPopulated
OverlayRegionIdDuplicate
OverlayShareClassEvictionUndefined
OverlayInstallSourceNotVisible
OverlayInstallEventDefaultMissing
OverlayCandidateNotInstalled
OverlayInstallReferencesUnknownRegion
OverlayInstallReferencesUnknownMember
OverlayLeaseShapeIncomplete
OverlayMemberPayloadExceedsRegion
OverlayCanonicalSortDrift
OverlayReportRoundTripFailed
OverlaySectionRoleLeaked
OverlaySchedulingFieldLeaked
OverlayRepairProvenanceForbidden
OverlayResolvedPolicyProjectionMismatch
OverlayTargetProfileLayoutUnsupported
OverlayNoCandidatesButReservationDeclared
```

Each code carries:

```text
ValidationDiagnostic {
    severity:    Hard,
    origin:      OverlayPlanConstruction,
    code:        OverlayPlan-specific,
    detail:      typed renderable record,
    provenance:  ValidationProvenance,
}
```

`detail` is a typed record with named fields, never a free-form String
(per `D-NoStringOnly`).

### 14.3 ArenaPlan diagnostic codes

```text
ArenaInputHashMismatch
ArenaAllocationFailed
ArenaCapacityExceeded
ArenaUnmappedStorageClass
ArenaLifetimeClassMismatch
ArenaAliasClassDisagreement
ArenaAliasClassMustOverlapDisagreement
ArenaSlotIdDuplicate
ArenaPersistentPageGeometryMismatch
ArenaPersistentPageStreamMismatch
ArenaCrossStreamPageSharing
ArenaSramSpanForbidden
ArenaHarnessLeakDetected
ArenaContinuationRecordSizeMismatch
ArenaHramOutOfRange
ArenaBank0WramOverflow
ArenaHramUsableCapExceeded
ArenaOverlayReservationOverflow
ArenaOverlayReservationUnderflow
ArenaOverlayReservationOverlap
ArenaOverlayReservationCountMismatch
ArenaOverlayReservationCapDrift
ArenaCanonicalSortDrift
ArenaReportRoundTripFailed
ArenaSectionRoleLeaked
ArenaSchedulingFieldLeaked
ArenaRepairProvenanceForbidden
ArenaTargetProfileLayoutUnsupported
ArenaPureExpressionAllocated
ArenaTraceRingMisplaced
ArenaPolicyProjectionMismatch
ArenaCertAddressInvariantFailed
```

### 14.4 Diagnostic laws

Inherited from F-B2/F-B4 §5:

```text
D-CodeClosed:
  Every diagnostic carries a typed enum code; no free-form ad-hoc codes.

D-NoStringOnly:
  Diagnostic detail is a typed record. String fields are allowed only
  for human-readable explanations whose machine-readable companion
  fields exist alongside.

D-Renderable:
  Every diagnostic detail must serialize to canonical JSON and be
  renderable in CLI output.

D-Provenance:
  Every diagnostic carries provenance back to a hash-bound input or
  policy source.
```

This RFC adds:

```text
D-OverlayPlanOriginExclusive:
  OverlayPlan diagnostics use origin = OverlayPlanConstruction; never
  another origin. This makes log filtering precise.

D-ArenaPlanOriginExclusive:
  ArenaPlan diagnostics use origin = ArenaPlanConstruction; never
  another origin.

D-NoSoftHere:
  No OverlayPlan or ArenaPlan diagnostic has severity = Soft (per
  R-HardOnly-ThisChunk).

D-RepairProvenanceForbidden:
  ∀ d ∈ overlay_plan.report.diagnostics ∪ arena_plan.report.diagnostics.
    d.provenance.policy_source ∉ {RepairProposal(_)}
   ∧ d.provenance.constraint_op ∉ {AuthorizedRelaxation(_)}
```

## 15. Cross-stage interactions

### 15.1 F-B8 (StoragePlan) handshake

`OverlayPlan` and `ArenaPlan` both consume `StoragePlanProduct`
verbatim. Neither stage rewrites:

```text
- StorageBinding.materialization
- StorageBinding.alias_class
- StorageBinding.lifetime
- StorageClass discriminant

Reads only:
- StorageBinding.id, .value, .materialization, .alias_class, .lifetime
- AliasClassRelations (MustOverlap, MayOverlap, MustDisjoint)
```

If F-B8 amends `LifetimeClass`, `StorageClass`, or `AliasClassId`, that
amendment must explicitly amend §2.6 (LifetimeClass arena map) and §2.8
(AliasClass slot-share rule) here.

### 15.2 F-B9 (SramPagePlan) handshake

`ArenaPlan` consumes `SramPagePlanProduct`'s persistent-page records:

```text
- (PersistPageId, CommitGroupId, SequenceStreamId, payload_bytes)
- PersistentPageGeometry (header size, payload, commit_word size,
                          page_alignment)
- Page-rotation policy (which pages share an arena, double-buffer pair)
```

`SramPagePlan`'s spill-policy and commit-boundary fields are NOT
consumed by `ArenaPlan` v1; they are used by F-B13 for persistence
slicing. If F-B9 amends `PersistentPageGeometry`, that amendment must
explicitly amend §2.7 here.

### 15.3 F-B10 (RomWindowPlan) handshake

`OverlayPlan` consumes from `RomWindowPlanProduct`:

```text
- candidates: { kernels | luts | expert_fragments } where residency = WramOverlay
- For each candidate: payload_bytes, overlay_source, required_install_event
- simultaneously_visible_at(install_event.start) -- visibility predicate
- target_profile.wram_layout (overlay region constraints)
```

`ArenaPlan` does NOT consume RomWindowPlan directly except through
the input identity hash (recorded for audit). All overlay information
flows through `OverlayPlan`.

### 15.4 F-B13 (GbSchedIR) consumption

F-B13 consumes both products by hash:

```text
- ArenaPlan: every SchedSlice references ArenaSlot ids. F-B13 never
  carves a new slot or moves bytes. live_wram and live_sram on a
  SchedSlice are both `Vec<ArenaSlot>`.

- OverlayPlan: ResourceLeaseKind::Overlay(OverlayId) uses OverlayId
  verbatim. ResourceStateValidation uses OverlayInstall.lease_shape to
  mint LeaseIds and to prove balance.
```

If F-B13 introduces a slot variant not bound by ArenaPlan, that is a
F-B13 bug, not a F-B12 amendment.

### 15.5 F-B14 (ScheduleCostAnalysis) consumption

F-B14 reads `OverlayPlan.installs` to charge overlay-install cycle
cost. F-B14 does NOT decide which kernels overlay (that is F-B10);
F-B14 does NOT decide install timing (that is F-B13); F-B14 charges
the costs F-B11 and F-B13 jointly determine.

### 15.6 F-B15 (Backend) consumption

F-B15 consumes `ArenaPlan` for code emission:

```text
- Section addresses: ArenaInstance.byte_range becomes section
  symbols in `.sym` output.
- ArenaSlot ids: emit symbols `arena_<NamedArena>_<slot_role>`.
- Persistent-page geometry: PersistHeader / commit_word lay out at the
  arena's slot offsets.
- Continuation record: WramContinuationRecord arena binds the
  InferenceStateHeader symbol address and the continuation tail window.
- Harness command/result blocks: Sram*HarnessCommandBlock /
  Sram*HarnessResultBlock arenas bind harness symbols.
```

F-B15 does NOT change ArenaPlan; it only consumes addresses.

### 15.7 F-B16 (FeasibilityRefinementLoop) interface

F-B16 (BLOCKED on oracle question) will read `OverlayPlan` and
`ArenaPlan` Hard diagnostics to drive `RepairPolicy` and bump
`CompileKnobs`. Until F-B16 lands:

```text
- OverlayPlan and ArenaPlan reports MUST NOT contain RepairProposal
  provenance or AuthorizedRelaxation operations.
- CompileKnobs schema is wired through ResolvedCompilePolicy.
  v1-relevant overlay/arena knobs are read only; F-B11/F-B12 do not
  mutate them.
```

### 15.8 F-B17 (StageCache integration sweep)

F-A6.2 (StageCache infrastructure) is closed. K11 and K12 wire into
StageCache directly. The cross-cutting F-B17 chunk later may add a
uniform sweep, but no per-stage wiring is missing here.

### 15.9 F-C3 (ScheduleOracle) consumption

F-C3 reads `ArenaPlan.wram_arenas`, `ArenaPlan.sram_arenas`, and
`ArenaPlan.hram_assignments` to bind emulator harness state. The
schedule oracle's storage-geometry contract IS the named-arena set
emitted here. Any F-C3 amendment to add a new arena class requires
a `NamedArena` enum bump in this RFC.

### 15.10 Epic A (gbf-abi, gbf-runtime) interactions

```text
- gbf-abi::PersistHeader sizeof and alignment is consumed by §9.6.
- gbf-abi::PersistGroupCommit sizeof is consumed by §9.6.
- gbf-abi::InferenceStateHeader sizeof plus continuation tail bytes are
  consumed by AP-SC-15.
- gbf-abi::HarnessCommandBlock and HarnessResultBlock sizeofs are
  consumed by the harness arena slot allocation.
- gbf-runtime::banking BankLease/BankGuard ABI is named in
  OverlayLeaseShape.rom_bank_lease but not implemented here. F-B13
  binds concrete LeaseIds.
- gbf-runtime::persistence consumes the persistent-page geometry
  bytes ArenaPlan lays out; runtime is the only writer.
```

### 15.11 Pure-core invariant

```text
F-Pure-Core:
  build_overlay_plan_core and build_arena_plan_core are pure in:
    - The closed input domain (§8.1, §9.1).
    - All hash-bound input identity is captured in K11/K12.
    - No filesystem access, no clock, no env vars, no global state.
    - All randomness is forbidden (no RngSlot consumption here).
  Drivers (run_stage8_5, run_stage9) handle JSON emission, StageCache
  reads/writes, and certificate emission.
```

## 16. Task DAG, compressed

```text
Wave0 SchemaPrelude:
  T-B11.0 overlay_plan.v1 ReportEnvelope binding
  T-B12.0 arena_plan.v1   ReportEnvelope binding
  T-B12.0a arena.cert.v1  ReportEnvelope binding
  All depend on F-B2's ReportEnvelope/canonical-JSON/self-hash machinery.

Wave1 OverlayTypes (T-B11):
  T-B11.1  OverlayPlan + OverlayPlanIdentity types
  T-B11.2  OverlayId, OverlayResidentId, ShareClassId, InstallId newtypes
  T-B11.3  OverlayRegion + WramRegionConstraint enum
  T-B11.4  OverlayShareClass + EvictionPolicy enum
  T-B11.5  OverlayInstall + OverlayInstallEvent + OverlayLeaseShape
            + OverlaySource (RomBank + offset, LUT id)
  T-B11.6  OverlayReservation + ReservationEntry + ReservationKind enum
  T-B11.7  OverlayPlanPolicyProjection (closed, projection-only)

Wave2 OverlayConstruction (T-B11):
  T-B11.8  bind_regions subroutine (canonical sort + cap checks)
  T-B11.9  assemble_share_classes subroutine
  T-B11.10 schedule_installs subroutine + visibility predicate
  T-B11.11 compute_reservation subroutine
  T-B11.12 build_overlay_plan_core (pure) wiring
  T-B11.13 OP-SC-1..18 self-consistency validators
  T-B11.14 CanonicalSort class + overlay_plan_self_hash via DomainHash
  T-B11.15 overlay_plan.v1 schema + product-bearing report
            (body.result.product: OverlayPlan) + semantic validator + tests
  T-B11.16 StageCache key K11 (DomainHash form) + success + failure-memo
  T-B11.17 run_stage8_5 driver (JSON emit + cache wire)
  T-B11.18 fixture: synthetic single-region OverlayPlan +
            synthetic share-class OverlayPlan in fixtures/overlay_plan/
  T-B11.19 reject fixtures: every OP-Reject-* class
  T-B11.20 (optional) overlay.cert.v1 schema + emitter
            (gated by CompileKnobs.emit_overlay_cert)

Wave3 ArenaTypes (T-B12):
  T-B12.1  ArenaPlan + ArenaPlanIdentity types
  T-B12.2  NamedArena closed enum (v1 set: §2.4)
  T-B12.3  ArenaId, ArenaSlotId newtypes
  T-B12.4  ArenaInstance + MemoryBacking enum + ArenaAlignment +
            ZerofillPolicy enum
  T-B12.5  ArenaSlot + SlotBindingKind + SlotBindingRef enums
  T-B12.6  ArenaBindings (materialize_to_slot, persist_to_slot_pair,
            overlay_to_arena maps)
  T-B12.7  OverlayReservationHonor + ReservationHonorEntry types
  T-B12.8  ArenaPlanPolicyProjection (closed, projection-only)
  T-B12.9  PersistentPageGeometry binding type

Wave4 ArenaConstruction (T-B12):
  T-B12.10 arena_for table (LifetimeClass × StorageClass -> NamedArena
            family); closed, exhaustive match
  T-B12.11 First-Fit-Decreasing allocator with canonical sort key (§9.4)
  T-B12.12 Persistent-page geometry binder (§9.6)
  T-B12.13 honor_overlay_reservation subroutine (§9.7)
  T-B12.14 build_arena_plan_core (pure) wiring
  T-B12.15 AP-SC-1..25 self-consistency validators
  T-B12.16 CanonicalSort class + arena_plan_self_hash via DomainHash
  T-B12.17 arena_plan.v1 schema + product-bearing report
            (body.result.product: ArenaPlan) + semantic validator + tests
  T-B12.18 arena.cert.v1 schema + emitter
            (AddressInvariantsCertificate, ReservationHonor,
             PersistentPageGeometryCertificate, HarnessNoLeakCertificate)
  T-B12.19 StageCache key K12 (DomainHash form) + success + failure-memo
  T-B12.20 run_stage9 driver (JSON + cert emit + cache wire)
  T-B12.21 fixture: synthetic Materialize+Persist+Overlay ArenaPlan in
            fixtures/arena_plan/
  T-B12.22 reject fixtures: every AP-Reject-* class

Wave5 IntegrationAndReview:
  T-B11.21 + T-B12.23 cross-product fixture: K11/K12 cache pinning test
            (build twice, assert byte-identical reports)
  T-B11.22 + T-B12.24 cross-product fixture: K11 miss ⇒ K12 miss
  T-B12.25 F-B13 readiness fixture: ArenaPlan consumed by a synthetic
            SchedSlice that references ArenaSlot ids
  T-B11.23 review-packet sub-bundle for F-B11
  T-B12.26 review-packet sub-bundle for F-B12
```

DAG law:

```text
Wave0 → Wave1 → Wave2 → Wave3 → Wave4 → Wave5
F-B11 must merge before F-B12.
F-B12 must not import a real OverlayPlan from a non-merged F-B11 PR.
F-B13 (downstream chunk) gains explicit dependency edges to bd-140 and
        bd-3bw once F-B12 lands.
F-C3 (downstream) gains a soft dependency edge to bd-3bw for harness
        binding.
```

Feature merge law:

```text
T-B11.16 (K11) and T-B12.19 (K12) land together with their pure-core
   constructors (T-B11.12, T-B12.14) so caching is wired before any
   cross-stage tests rely on it.
T-B12.18 (arena.cert.v1) is required for chunk closure.
T-B11.20 (overlay.cert.v1) is optional; closure does not require it.
```

## 17. Rejection classes (closure gate)

This chunk closes only when every class below is exercised by a typed
fixture.

### 17.1 F-B11 reject classes

```text
OP-Reject-1:  OverlayInputHashMismatch                        -- §8.1
OP-Reject-2:  OverlayWramOverlayCapExceeded                   -- §8.6
OP-Reject-3:  OverlayRegionPayloadExceedsRegionCap            -- §8.6
OP-Reject-4:  OverlayRegionEmptyButPopulated                  -- §8.6
OP-Reject-5:  OverlayRegionIdDuplicate                        -- §8.6
OP-Reject-6:  OverlayShareClassEvictionUndefined              -- §8.4
OP-Reject-7:  OverlayInstallSourceNotVisible                  -- §8.5 / OP-SC-8
OP-Reject-8:  OverlayInstallEventDefaultMissing               -- §8.5
OP-Reject-9:  OverlayCandidateNotInstalled                    -- OP-SC-9
OP-Reject-10: OverlayInstallReferencesUnknownRegion           -- OP-SC-6
OP-Reject-11: OverlayInstallReferencesUnknownMember           -- OP-SC-7
OP-Reject-12: OverlayLeaseShapeIncomplete                     -- OP-SC-14
OP-Reject-13: OverlayMemberPayloadExceedsRegion               -- OP-SC-2
OP-Reject-14: OverlayCanonicalSortDrift                       -- OP-SC-15
OP-Reject-15: OverlayReportRoundTripFailed                    -- §12.6
OP-Reject-16: OverlaySectionRoleLeaked                        -- OP-SC-16
OP-Reject-17: OverlaySchedulingFieldLeaked                    -- OP-SC-17
OP-Reject-18: OverlayRepairProvenanceForbidden                -- OP-SC-18
OP-Reject-19: OverlayResolvedPolicyProjectionMismatch         -- §13.1
OP-Reject-20: OverlayTargetProfileLayoutUnsupported           -- OP-SC-3
OP-Reject-21: OverlayNoCandidatesButReservationDeclared       -- §8.3
```

### 17.2 F-B12 reject classes

```text
AP-Reject-1:  ArenaInputHashMismatch                          -- §9.1
AP-Reject-2:  ArenaAllocationFailed                           -- §9.4
AP-Reject-3:  ArenaCapacityExceeded                           -- §9.4
AP-Reject-4:  ArenaUnmappedStorageClass                       -- §9.5
AP-Reject-5:  ArenaLifetimeClassMismatch                      -- §2.6 / AP-SC-8
AP-Reject-6:  ArenaAliasClassDisagreement                     -- AP-SC-5/6
AP-Reject-7:  ArenaAliasClassMustOverlapDisagreement          -- §9.4
AP-Reject-8:  ArenaSlotIdDuplicate                            -- AP-SC-22
AP-Reject-9:  ArenaPersistentPageGeometryMismatch             -- AP-SC-9
AP-Reject-10: ArenaPersistentPageStreamMismatch               -- §9.6
AP-Reject-11: ArenaCrossStreamPageSharing                     -- §9.6
AP-Reject-12: ArenaSramSpanForbidden                          -- AP-SC-12
AP-Reject-13: ArenaHarnessLeakDetected                        -- AP-SC-13
AP-Reject-14: ArenaContinuationRecordSizeMismatch             -- AP-SC-15
AP-Reject-15: ArenaHramOutOfRange                             -- AP-SC-14
AP-Reject-16: ArenaBank0WramOverflow                          -- AP-SC-16
AP-Reject-17: ArenaHramUsableCapExceeded                      -- AP-SC-16
AP-Reject-18: ArenaOverlayReservationOverflow                 -- §11.3
AP-Reject-19: ArenaOverlayReservationUnderflow                -- §11.3
AP-Reject-20: ArenaOverlayReservationOverlap                  -- §11.3
AP-Reject-21: ArenaOverlayReservationCountMismatch            -- §11.3
AP-Reject-22: ArenaOverlayReservationCapDrift                 -- §11.3
AP-Reject-23: ArenaCanonicalSortDrift                         -- AP-SC-20
AP-Reject-24: ArenaReportRoundTripFailed                      -- §12.6
AP-Reject-25: ArenaSectionRoleLeaked                          -- AP-SC-17
AP-Reject-26: ArenaSchedulingFieldLeaked                      -- AP-SC-18
AP-Reject-27: ArenaRepairProvenanceForbidden                  -- AP-SC-19
AP-Reject-28: ArenaTargetProfileLayoutUnsupported             -- §9.5
AP-Reject-29: ArenaPureExpressionAllocated                    -- AP-SC-21 / AP-SC-4
AP-Reject-30: ArenaTraceRingMisplaced                         -- AP-SC-23
AP-Reject-31: ArenaPolicyProjectionMismatch                   -- §13.2
AP-Reject-32: ArenaCertAddressInvariantFailed                 -- §12.3
```

Each reject class is gated by a typed fixture under
`fixtures/overlay_plan/reject/` or `fixtures/arena_plan/reject/`.

## 18. Proof obligations

```text
O1 OverlayPlan / ArenaPlan determinism:
  Same inputs produce byte-identical overlay_plan.json and
  arena_plan.json across two clean regenerations.

O2 Self-hash + product round-trip:
  Both reports and their embedded products round-trip through
  parse → canonicalize → semantic validation → self-hash.

O3 OverlayPlan rejection completeness:
  Every OP-Reject-* class has a fixture and typed diagnostic.

O4 ArenaPlan rejection completeness:
  Every AP-Reject-* class has a fixture and typed diagnostic.

O5 Reservation handshake (load-bearing):
  ∀ inputs.
    HonorPredicate(OverlayPlan(inputs).reservation,
                   ArenaPlan({inputs, OverlayPlan(inputs)}).overlay_reservation)
    holds, OR ArenaPlan fails with a typed reservation diagnostic.

O6 Address invariants (load-bearing):
  Every F-Addr-1..14 invariant is provable from the joint product
  (OverlayPlan, ArenaPlan), and arena.cert.v1 records each as a
  boolean flag set true on success.

O7 LifetimeClass preservation:
  ∀ b ∈ sp.bindings with b.materialization = Materialize{lifetime: L}.
    arena_bindings.materialize_to_slot[b.id].lifetime_class = L

O8 AliasClass slot-share rule:
  ∀ s1, s2 ∈ ap.slots(*) with s1.byte_range overlaps s2.byte_range
                          ∧ s1.arena_id = s2.arena_id.
    s1 = s2 ∨ (s1.alias_class_id = s2.alias_class_id ∧
               s1.byte_range = s2.byte_range)

O9 OverlayPlan coverage:
  Every WramOverlay candidate from RomWindowPlan has an OverlayInstall
  whose member equals the candidate id.

O10 OverlayPlan source-visibility:
  Every install's source ROM bank is visible at the install_event start
  per RomWindowPlan's simultaneously_visible_at predicate.

O11 Persistent-page geometry conforms to runtime ABI:
  Each (PersistPageId, CommitGroupId) is bound to two slots whose
  size = sizeof(PersistHeader) + payload_bytes(page) + sizeof_commit_word,
  in the SramSequenceStatePages(stream_id_of(page)) arena, with valid
  alignment.

O12 Harness arena no-leak:
  SramHarnessCommandBlock and SramHarnessResultBlock arenas are
  disjoint from every SramSequenceStatePages, SramPersistedTranscript,
  SramColdSpill, and SramTracePages slot.

O13 Continuation record matches gbf-abi::InferenceStateHeader + tail:
  ap.wram_arenas.lookup(WramContinuationRecord).byte_range.len
    = sizeof_repr_c(InferenceStateHeader) + continuation_tail_window_bytes,
  with arena.cert.v1 recording header slot, tail slots, byte offsets,
  sizes, and source ValueIds.

O14 Bank0 WRAM/HRAM caps honored:
  - Σ bank0-WRAM arena bytes ≤ rcb.wram_runtime_floor_bytes.
  - Σ HRAM assignment bytes ≤ rcb.hram_usable_bytes.
  - Σ overlay reservation bytes ≤ rcb.wram_overlay_cap_bytes.

O15 Pure expression no-allocation:
  No Recompute binding has an arena slot. No pure-expression ValueId
  appears in arena_bindings.materialize_to_slot.

O16 Cache soundness:
  - Failure memos are never replayed as success products.
  - K11 / K12 hits replay byte-identical products and reports.
  - K11 / K12 miss correctly on pass_version, schema, feature-set, or
    upstream self-hash drift.
  - K12 misses whenever K11 misses (no stale OverlayPlan reuse).

O17 No hidden defaults:
  Every plan field derives from a hash-bound input or fails loudly. No
  silent default fills a missing region size, eviction policy, install
  event, arena alignment, or zerofill policy.

O18 No scheduling fusion / no codegen leakage:
  Neither OverlayPlan nor ArenaPlan carries SliceId, LeaseId,
  ResourceVector, CycleBudget, YieldKind, ExitKind, SectionRole,
  AsmIR, or any byte-level payload. OverlayInstallEvent is a static
  descriptor; ArenaInstance.byte_range is offset/length only.

O19 No repair provenance:
  Stage 8.5 and Stage 9 reports do not contain RepairProposal source
  or AuthorizedRelaxation operations.

O20 Pure-function shape:
  build_overlay_plan_core and build_arena_plan_core are pure functions
  of their typed inputs. Side effects (JSON, cache, certs) are
  isolated in the run_stageN drivers.

O21 NamedArena closed in v1:
  Every ArenaInstance.named ∈ NamedArena (§2.4). No stage may construct
  an ArenaInstance with a NamedArena variant outside the v1 set.

O22 Canonical sort stable:
  CanonicalSort applied twice equals applied once. Self-hash is
  invariant under iteration-order reshuffling within unsorted inputs.

O23 Coverage of materialize bindings:
  Every Materialize StorageBinding has exactly one ArenaSlot; every
  Persist StorageBinding has exactly one (slot_a, slot_b) pair;
  every WramOverlay residency choice has an OverlayInstall.

O24 ArenaSlotId uniqueness:
  ArenaSlotIds are unique across all arenas.

O25 OverlayPlan ⊆ ArenaPlan reservation:
  ArenaPlan.overlay_reservation.per_region equals
  OverlayPlan.reservation.per_region under the canonical sort, with
  byte_range filled in by ArenaPlan.

O26 F-B13 readiness:
  Every ArenaSlot id, NamedArena name, and OverlayId is stable across
  runs and recoverable by hash; F-B13 may consume them by reference
  without re-deriving them.

O27 F-C3 readiness:
  Every NamedArena variant in ap.* corresponds to a binding the
  ScheduleOracle's emulator harness can reproduce; ScheduleOracle's
  storage geometry is ArenaPlan's named-arena set verbatim.

O28 PersistHeader / PersistGroupCommit ABI freshness:
  ArenaPlan derives page_size from current sizeof(PersistHeader) and
  sizeof(PersistGroupCommit) at compile time, not from a frozen
  constant. If gbf-abi bumps the layout, ArenaPlan recomputes.

O29 Hashing convention:
  All hashes use F-B2/F-B4's DomainHash convention. Bitwise mixing of
  sub-hashes is forbidden.

O30 No partial product on failure:
  Failed OverlayPlan / ArenaPlan reports have body.result = None;
  no certificate is emitted on failure.

O31 Diagnostic taxonomy:
  Every diagnostic uses ValidationOrigin = OverlayPlanConstruction or
  ArenaPlanConstruction; every code is a typed enum variant; every
  detail is a typed renderable record.

O32 Cross-stream isolation:
  Persistent pages from one SequenceStreamId never share an arena with
  pages from another stream.

O33 ArenaCertAddressInvariantFailed -> failed plan:
  If any AddressInvariantsCertificate flag is false, ArenaPlan
  construction must fail with ArenaCertAddressInvariantFailed and emit
  no certificate (because a failed plan emits no cert).

O34 OverlayId / ArenaId / ArenaSlotId stability:
  These ids are stable across runs on identical inputs. Reordering
  unsorted input collections does not change them.

O35 OverlayInstall.lease_shape totality:
  Every install carries a complete OverlayLeaseShape with both
  rom_bank_lease and wram_region_lease populated.

O36 No global state leakage:
  Pure cores observe no environment, time, RNG, or filesystem. The
  only inputs are §8.1 / §9.1 records.
```

## 19. End-to-end theorem

```text
Theorem SpatialPlanPipelineSoundness:

Given:
  Imported inputs i.
  validate_artifact_and_request(i) = Ok(v)                  [F-B2]
  resolve_policy(v)                = Ok(p)                  [F-B2]
  build_quant_graph(...)           = Ok(q)                  [F-B3]
  static_budget(...)               = Ok(b) ∧ b.fits = true  [F-B4]
  build_infer_ir(...)              = Ok(g)                  [F-B5]
  build_observation_plan(...)      = Ok(obs)                [F-B6]
  build_range_plan(...)            = Ok(rng)                [F-B7]
  build_storage_plan(...)          = Ok(sp)                 [F-B8]
  build_sram_page_plan(...)        = Ok(spp)                [F-B9]
  build_rom_window_plan(...)       = Ok(rwp)                [F-B10]
  build_overlay_plan_core({sp, spp, rwp, p, rcb, target_profile})
                                   = Ok(op)                 [F-B11]
  build_arena_plan_core({sp, spp, rwp, op, p, rcb, target_profile})
                                   = Ok(ap)                 [F-B12]

Then:
  1. op is a valid OverlayPlan:
     - Region/install/share-class triad is canonically sorted, hash-bound,
       and content-addressed.
     - Every WramOverlay candidate from rwp has at least one OverlayInstall.
     - Every install's source bank is visible at install_event start
       per rwp.simultaneously_visible_at.
     - OverlayLeaseShape is complete on every install.
     - OverlayReservation total_bytes ≤ rcb.wram_overlay_cap_bytes.
     - Per-region reservation bytes ≤ rcb.wram_overlay_region_max_bytes.
     - Every region member's payload size ≤ region.bytes.
     - No section role, no AsmIR, no scheduling field, no repair
       provenance.

  2. ap is a valid ArenaPlan:
     - Every Materialize StorageBinding maps to exactly one ArenaSlot.
     - Every Persist StorageBinding maps to exactly one
       (PersistentPageA, PersistentPageB) slot pair.
     - LifetimeClass is preserved from StoragePlan to every slot.
     - LifetimeClass admits the slot's NamedArena per §2.6.
     - AliasClass equality on overlapping byte ranges; AliasClass
       inequality enforces disjointness.
     - Persistent-page geometry conforms to PersistHeader + payload +
       commit_word at the runtime ABI's current sizeof.
     - SRAM arenas do not span 8 KiB bank boundaries.
     - HRAM assignments live within [hram_base, hram_base + 0x80).
     - WramContinuationRecord arena byte_range.len equals
       sizeof_repr_c(InferenceStateHeader) plus the continuation tail
       window, with arena.cert.v1 tail-slot witnesses.
     - SramHarnessCommandBlock / SramHarnessResultBlock disjoint from
       every model-data SRAM arena.
     - Σ Bank0-WRAM bytes ≤ rcb.wram_runtime_floor_bytes.
     - Σ HRAM bytes ≤ rcb.hram_usable_bytes.
     - No section role, no AsmIR, no scheduling field, no repair
       provenance, no pure expression has a slot.

  3. The reservation handshake holds (load-bearing):
     HonorPredicate(op.reservation, ap.overlay_reservation) is true.
     ∀ r ∈ op.regions.
       ∃! a ∈ ap.wram_arenas with a.named = WramOverlayRegion(r.id)
                                ∧ a.byte_range.len = r.bytes
                                ∧ a.byte_range disjoint from every
                                  non-overlay WRAM arena.

  4. Every materialized value has a deterministic, lifetime-correct
     address:
       For every value V with sp.binding(V).materialization = Materialize{L}:
         let slot := ap.materialize_to_slot[binding_id(V)]
         slot is in some ArenaInstance whose backing matches V's
         allowed memory class; slot.lifetime_class = L; slot's byte
         offset within its arena is deterministic and stable across
         runs.
     For every value V with sp.binding(V).materialization = Persist{p, cg}:
         let (slot_a, slot_b) := ap.persist_to_slot_pair[(p, cg)]
         both slots are in arena SramSequenceStatePages(stream_id_of(p)),
         their byte_ranges are disjoint, both sized for the
         PersistHeader + payload + commit_word geometry, the active
         page is selected by the runtime's PersistGroupCommit (not by
         this pass).

  5. Overlay reservations are exactly honored: no overflow, no
     underflow, no unrelated WRAM arena steals from a reserved region.

  6. Both products are content-addressed and reproducible across two
     consecutive regenerations on the same upstream hashes:
       op.self_hash and ap.self_hash are stable.
       overlay_plan.json and arena_plan.json round-trip byte-identically.
       arena.cert.json round-trips byte-identically.

  7. F-B13 (GbSchedIR) may consume op and ap by hash without
     re-deriving any byte range or overlay region; F-B14
     (ScheduleCostAnalysis) may charge install costs against
     op.installs; F-B15 (Backend) may emit section symbols against
     ap.byte_ranges; F-C3 (ScheduleOracle) may bind emulator harness
     state to ap.named arenas.

  8. The persistent-record protocol's compile-time witness is in place:
     every Persist binding has a typed double-buffered slot pair whose
     geometry conforms to gbf-runtime::persistence's writer/reader
     contract. The runtime later validates against the same shape;
     this pass produces the addresses.

  9. The harness arena is isolated: SRAM crashes or malformed harness
     writes cannot contaminate sequence-state recovery.

Not proven:
  - Slice scheduling                 (F-B13)
  - Lease balance / yield safety     (F-B13 ResourceStateValidation)
  - Cycle costs                      (F-B14)
  - Section ordering / reachability  (F-B15)
  - Refinement-loop convergence      (F-B16)
  - Conformance against ConformanceEnvelope (F-C4)
  - Persistence runtime mutation     (gbf-runtime::persistence)
  - Overlay byte transport runtime   (gbf-runtime::banking)
  - Trace data production            (Epic D)
```

## 20. Final concise contract

```text
F-B11 / F-B12 is correct when:

1. OverlayPlan is a deterministic pure function of pinned hash-bound
   inputs (StoragePlanProduct, SramPagePlanProduct, RomWindowPlanProduct,
   OverlayPlanPolicyProjection, RuntimeChromeBudget, TargetProfileSummary).
   Same inputs yield byte-identical overlay_plan.json. The pure core
   never does IO; the run_stage8_5 driver isolates JSON emission and
   StageCache writes.

2. ArenaPlan is a deterministic pure function of pinned hash-bound
   inputs (StoragePlanProduct, SramPagePlanProduct, RomWindowPlanProduct,
   OverlayPlanProduct, ArenaPlanPolicyProjection, RuntimeChromeBudget,
   TargetProfileSummary). Same inputs yield byte-identical arena_plan.json
   and arena.cert.json.

3. Region-first OverlayPlan: bytes belong to regions; installs own
   time; share classes own region-sharing equivalence. No install holds
   bytes a region does not already own. Two installs targeting the same
   region time-share the bytes via OverlayShareClass.

4. NamedArena is closed in v1 (§2.4). Every ArenaInstance.named is one
   of the 17 v1 variants. New variants require RFC amendment plus a
   schema bump.

5. LifetimeClass is preserved verbatim from StoragePlan to every
   ArenaSlot. The (LifetimeClass × StorageClass) → NamedArena family
   map (§2.6) is closed and exhaustive.

6. AliasClass slot-share rule: two slots with the same arena_id and
   overlapping byte_range have the same alias_class_id and identical
   byte_range; otherwise byte_ranges are disjoint.

7. Every Materialize StorageBinding has exactly one ArenaSlot. Every
   Persist StorageBinding has exactly one (PersistentPageA,
   PersistentPageB) slot pair in the SramSequenceStatePages arena for
   that page's stream. Recompute / pure-expression bindings have no
   slot.

8. Persistent-page geometry conforms to PersistHeader + payload +
   commit_word at gbf-abi's current sizeof. Pages of the same kind
   for the same stream are double-buffered as (page A, page B) in one
   arena; the active page is selected by the runtime's PersistGroupCommit.

9. The reservation handshake is exact: OverlayPlan reserves WRAM bytes
   per region; ArenaPlan binds one WramOverlayRegion(r.id) arena per
   region whose byte_range.len = r.bytes and whose byte_range is
   disjoint from every non-overlay WRAM arena. Over- and
   under-reservation are typed Hard rejects.

10. Bank0 budgets honored:
    - Σ Bank0-WRAM bytes ≤ rcb.wram_runtime_floor_bytes
    - Σ HRAM bytes ≤ rcb.hram_usable_bytes
    - Σ overlay reservation bytes ≤ rcb.wram_overlay_cap_bytes
    Continuation record byte_range.len = sizeof_repr_c(InferenceStateHeader)
    plus the continuation tail window.
    Harness command/result blocks live in dedicated SRAM arenas
    disjoint from sequence-state, transcript, cold-spill, and trace
    arenas.

11. SRAM arenas do not span 8 KiB bank boundaries. HRAM assignments
    live within [hram_base, hram_base + 0x80) with byte alignment.

12. Both reports are canonical and content-addressed:
    overlay_plan_self_hash and arena_plan_self_hash use the F-B2/F-B4
    DomainHash convention; bitwise mixing of sub-hashes is forbidden.
    Failed reports have body.result = None; no certificate is emitted
    on failure. `arena.cert.json` is required on success;
    `overlay.cert.json` is optional ("Amends planv0").

13. StageCache keys K11 and K12 use DomainHash; cache miss occurs on
    pass_version, schema, feature-set, or any input self-hash drift;
    cache hit replays byte-identical canonical product and report.
    K11 misses imply K12 misses (overlay_plan_self_hash drift
    propagates).

14. Diagnostics use the closed origin set ValidationOrigin = {
    OverlayPlanConstruction, ArenaPlanConstruction }. Every code is a
    typed enum variant; every detail is typed and renderable. No Soft
    diagnostics in this chunk. No RepairProposal source or
    AuthorizedRelaxation operation in any diagnostic provenance —
    F-B16 introduces those surfaces by amendment in a later RFC.

15. Pure-function shape is preserved: build_overlay_plan_core and
    build_arena_plan_core do no IO and observe no global state.
    run_stage8_5 and run_stage9 drivers are the only IO surfaces and
    are the only places StageCache reads/writes occur.

16. F-B13 readiness: every OverlayId, ArenaId, ArenaSlotId, NamedArena
    name is stable across runs on identical inputs and recoverable by
    hash. F-B13 references slots and overlay regions by id; it neither
    carves new slots nor moves bytes.

17. F-B14 readiness: OverlayInstall.install_event and lease_shape are
    sufficient for ScheduleCostAnalysis to charge install cycle cost
    against calibration without re-deriving overlay membership.

18. F-B15 readiness: ArenaInstance.byte_range is the address contract
    the backend's section emitter consumes. Section roles, far-call
    legalization, branch relaxation, and bank placement are F-B15's;
    address ranges are F-B12's.

19. F-C3 readiness: NamedArena's v1 closed set IS the storage geometry
    ScheduleOracle binds in emulator harness mode. Any new variant
    requires explicit RFC amendment.

20. F-B16 hooks: CompileKnobs schema (resolved by F-B2 Stage 0.5)
    carries any overlay-/arena-relevant knobs as named-only hooks. No
    knob is consumed by F-B11 or F-B12 in v1 beyond reading the
    resolved value. F-B16 unblocks RepairProposal source by amendment.

21. Address invariants F-Addr-1..14 (§10) hold over the joint product
    (OverlayPlan, ArenaPlan); arena.cert.v1's
    AddressInvariantsCertificate records each as a boolean flag set
    true on success. A false flag is impossible in a passing
    arena_plan.json (failed plan emits no cert).

22. Schema versioning: overlay_plan.v1, arena_plan.v1, arena.cert.v1,
    and (optional) overlay.cert.v1 follow F-B2/F-B4 §10 evolution
    rules. Cross-major schema changes require a new RFC and a
    StageCache key migration.
```
