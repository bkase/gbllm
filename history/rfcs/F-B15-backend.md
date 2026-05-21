# RFC F-B15: Backend (Stage 12) — `AsmIR` codegen + `ReachabilityValidation` + `PlacedRom` + `EncodedRom`

## -1. Authority and amendment policy

This RFC is the source of truth for F-B15 implementation. `history/planv0.md`
remains the architectural context document, but this RFC is allowed to refine,
narrow, or supersede `planv0.md` wherever this RFC makes a more precise
implementation decision.

Whenever this RFC intentionally diverges from `planv0.md`, the divergence must
be recorded in an `Amends planv0` note close to the relevant decision. This is
not a request to edit `planv0.md` immediately; it is a local source-of-truth
ledger for reviewers and implementers.

Rules:

* If this RFC and `planv0.md` disagree on F-B15 behavior, this RFC wins.
* If this RFC is silent, use `planv0.md` as context, not as a hidden
  acceptance gate.
* If this RFC and `F-B2-F-B4-pipeline-entry-validation.md` disagree on a
  shared surface (canonical JSON rule, self-hash convention, diagnostic
  envelope, StageCache key construction, `ReportEnvelope` shape), the
  F-B2/F-B4 RFC wins. F-B15 inherits those surfaces unchanged unless this
  RFC explicitly amends them.
* If this RFC and `F-B3-F-B5-canonical-irs.md` disagree on the
  pure-core/driver split, canonical-product handling, or the
  `Hash256`/`DomainHash`/`SelfHash` rules, the F-B3/F-B5 RFC wins.
* If this RFC and `F-B11-F-B12-overlay-arena-plans.md` disagree on overlay
  install events, arena byte-range geometry, or persistent-page identity,
  the F-B11/F-B12 RFC wins.
* If this RFC and `F-A1-gbf-asm.md` disagree on `AsmIR` shape, `Section`
  taxonomy, encoder behavior, layout/relax fixed point, or symbol naming,
  **the F-A1 RFC wins**. F-A1 owns the AsmIR types, the encoder, and the
  layout/relax/legalization machinery. F-B15 owns the **codegen front-end
  pipeline that produces `AsmIR` sections from a `SchedulePack`**, and the
  **whole-program analyses (`ReachabilityValidation`, `PlacedRom`,
  `EncodedRom`) that consume those sections** alongside Bank0 nucleus
  sections from F-A5 and the cartridge header from F-A1's ROM builder.
* If this RFC and `F-A2-gbf-hw.md` disagree on memory regions, MBC5
  register addresses, the cartridge-header constants, the `PpuMode` table,
  or the calibration schema, the F-A2 RFC wins.
* If this RFC and `F-A3-gbf-abi.md` disagree on `BuildIdentityBlock`,
  `CompatibilityEnvelope`, `SemanticCheckpointSchema`, the harness block
  shapes, `LivenessCounters` layout, or the `FaultCode`/`FaultDomain`
  partition, the F-A3 RFC wins.
* If this RFC and `F-A4-banklease-banking.md` disagree on the BankLease
  /BankGuard ABI surface, the per-call lowering shape, the HRAM shadow
  ownership, or the `InterruptSafetyTable` declaration substrate, the
  F-A4 RFC wins.
* If this RFC and `F-A5-bank0-runtime.md` disagree on Bank0 nucleus
  ownership, vector-stub residency, the `RuntimeShellModule` enum,
  `ExecutionContext`/`InterruptDiscipline` annotations, or the
  `runtime_nucleus_hash` normalization, the F-A5 RFC wins.
* If this RFC and `F-B13` (GbSchedIR + ResourceStateValidation) disagree on
  the `SchedulePack` shape, `SchedSlice`/`SchedOp` semantics, the
  continuation ABI, or the `ResourceLease`/`ResourceVector` accounting,
  the F-B13 RFC wins. F-B15 consumes `SchedulePack` by hash and treats it
  as a frozen input.
* If this RFC and `F-B14` (`ScheduleCostAnalysis`) disagree on cost
  envelope shape, the F-B14 RFC wins.
* If a later RFC changes any public type, report shape, cache key,
  diagnostic code, or canonicalization rule introduced here, that later
  RFC must explicitly amend this RFC.
* Source-of-truth changes must be expressed as typed schema changes, not
  prose folklore.

| Field           | Value |
|-----------------|-------|
| Author          | bkase / canonicalized by design pass |
| Status          | Draft |
| Feature bead    | bd-18d **F-B15 Backend (Stage 12)** |
| Open tasks      | To be minted: T-B15.1..T-B15.N (one task group per sub-pass — see §15) |
| Closed tasks    | None |
| Plan reference  | `history/planv0.md` lines 1906–1985 (Stage 12 Backend body — `AsmIR`, `ReachabilityValidation`, `PlacedRom`, `EncodedRom`); 1770–1900 (Stage 10 `GbSchedIR` — input); 1894–1906 (Stage 11 `ScheduleCostAnalysis` — input); 1985–2080 (BuildReports — `map.json`, `.sym`, `.lst`, `reachability_report.json`, `certs/reachability.cert.json`); 1989–2210 (runtime architecture, banking, persistence); 113–212 (the GB target — regions, banks, MBC5); 2466–2640 (Assembly eDSL — `gbf-asm`, codegen front-end); 2539–2640 (Profiles and objectives — placement profiles); 2640–2870 (tests, certs, reports/artifacts) |
| Glossary        | `history/glossary.md` (residency, common bank, expert bank, ISR-reachable, BankLease, Bank0, vector slot, far-call thunk, placement profile, reachability class, privilege class, lease, machine effect) |
| Constitution    | §I correctness by construction; §II three-stratum oracle correspondence; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |
| Companion RFCs  | F-B2/F-B4 Pipeline Entry & Validation; F-B3/F-B5 Canonical IRs; F-B11/F-B12 Spatial Plans; F-B13 GbSchedIR + ResourceStateValidation (input); F-B14 ScheduleCostAnalysis (consumed for budget annotations); F-B16 FeasibilityRefinementLoop (downstream — may request retry under different placement profile); F-A1 gbf-asm (AsmIR types, encoder, layout/relax, ROM builder); F-A2 gbf-hw (TargetProfile, region map, MBC5 invariants); F-A3 gbf-abi (BuildIdentityBlock, CompatibilityEnvelope, SemanticCheckpointSchema, far-call ABI); F-A4 BankLease/BankGuard ABI; F-A5 Bank0 runtime + interrupt vectors; F-A7 gbf-emu (consumes encoded ROM); F-A8 gbf-debug (consumes `.sym` + `.lst` + `map.json`); F-F2 Certificates (consumes `certs/reachability.cert.json`) |
| Sister deps     | bd-3s0s (T-A1x.2 — make interrupt vectors `ReachabilityValidation` roots — blocks on F-A1 vector-first-class layout work); bd-txth (F-F2 — Certificates: range, arena, window, reachability, resource_state — depends on `certs/reachability.cert.json` shape pinned here) |

## 0. Where this chunk lives — project, Epic B, and pipeline placement

This section orients the reader: where F-B15 sits inside the
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
          contracts every other epic builds on. Substantially merged on
          main: F-A1 (gbf-asm), F-A2 (gbf-hw), F-A3 (gbf-abi), F-A4
          (BankLease/BankGuard), F-A6 (gbf-store + StageCache) closed;
          F-A5 (Bank0 runtime), F-A7 (gbf-emu), F-A8 (gbf-debug) in flight.

Epic B — Compiler Pipeline (14 stages + refinement loop)        ← THIS EPIC
          The transform pipeline from frozen ArtifactCore +
          CompileRequest to a CompiledBuild (ROM + reports + certificates).
          Where most of M1–M3 lives, and where F-B15 closes the M2
          milestone's headline backend deliverable.

Epic C — Oracle Stack
          DenotationalOracle (F-C1), ArtifactOracle (F-C2),
          ScheduleOracle (F-C3), ConformanceEnvelope (F-C4).
          Defines the three-stratum correspondence relation that proves
          the deployed ROM behaves like the trained model.

Epic D — Runtime Beyond M0
          Persistence, harness, trace, drift, fault, SchedulePack
          mode-switch policy.

Epic E — Calibration & Bench
          gbf-bench production: cycle calibration, kernel timing, autotune.

Epic F — Reports & Verify
          gbf-report (build reports, certificates) + gbf-verify (independent
          slow reference implementations).

Epic G — Data, Lexical, Decode Pipeline
          gbf-data (corpus, charset, normalization, decode policy).

Epic H — Kernel
          gbf-kernel (KernelSpec + matvec/residual/norm/route/decode kernel
          implementations).
```

The training side is a separate epic-level bundle (`bd-1rb` Training-Contract
Revision Pass) that produces the `ArtifactCore` Epic B consumes.

### 0.2 Epic B's anatomy — the 14-stage pipeline plus loop

Epic B (`bd-2bw`) is the compiler. Per `planv0.md` §"The compiler pipeline,"
it has 14 numbered stages bracketed by a **policy/feasibility envelope**, a
**transform pipeline**, and a **reporting envelope**, plus a bounded
`FeasibilityRefinementLoop` that wraps stages 5–11.

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
  F-B13 Stages 10/10.5 GbSchedIR + ResourceStateValidation
  F-B14 Stage 11       ScheduleCostAnalysis
  F-B15 Stage 12       Backend                                       ← THIS RFC
                       (AsmIR + ReachabilityValidation +
                        PlacedRom + EncodedRom)

Cross-cutting:
  F-B16 FeasibilityRefinementLoop + RepairPolicy + CompileKnobs
        (BLOCKED on oracle question — may request that F-B15 retry
         under a different PlacementProfile if PackedExperts could not
         pack)
  F-B17 StageCache integration sweep across all stages
        (uniformization pass; F-B15 wires K12/K13/K14/K15 directly here)
```

Sequencing of weekly chunks (bkase 2026-05-07 conversation):

```text
Chunk 1:               F-B2 + F-B4         Stages 0, 0.5, 2
Chunk 2:               F-B3 + F-B5         Stages 1, 3
Chunk 3:               F-B6 + F-B7         Stages 4, 5
Chunk 4:               F-B8                Stage 6
Chunk 5:               F-B9 + F-B10        Stages 7, 8
Chunk 6:               F-B11 + F-B12       Stages 8.5, 9
Chunk 7:               F-B13               Stages 10, 10.5
Chunk 8:               F-B14 + F-B17       Stage 11 + cache wiring
Chunk 9 (THIS RFC):    F-B15               Stage 12 (large; may overflow)
Chunk 10 (oracle):     F-B16               Refinement loop
```

This is the largest single chunk in the pipeline and the only chunk that
explicitly carries an "may overflow" warning in the sequencing table. The
load-bearing reason is structural: Stage 12 contains **four** internal
sub-passes whose contracts cannot be split across chunks without breaking
the typed seam between AsmIR codegen, whole-program reachability,
placement, and encoding. §1.4 explains why splitting was rejected.

### 0.3 Where F-B15 sits in the pipeline

F-B15 is the **last transformative stage** and the **headline backend
chunk** of Epic B. It consumes a frozen `SchedulePack` (F-B13's output)
plus a frozen `ScheduleCostReport` (F-B14's output) plus the resolved
`TargetProfile` (F-A2), the resolved `CompileRequest` /
`ResolvedCompilePolicy` (F-B2's output), the runtime nucleus sections
(F-A5's output), the cartridge header (F-A1/F-A2's output), and produces:

* **AsmIR sections** for every compiled `SchedSlice` plus the per-mode
  scheduler glue (continuation entry, far-call thunks, vector stubs);
* **a `ReachabilityReport`** classifying every byte of code/data into one
  of six reachability classes (§9.1) and validating the seven hard rules
  enumerated in `planv0.md` line 1934+;
* **a `PlacedRom`** assigning every section to a concrete bank + start
  address under the resolved `PlacementProfile`, with branches relaxed,
  far-call thunks inserted, and bank-switch sequences coalesced;
* **an `EncodedRom`** consisting of `.gb` (the cartridge), `.sym` (the
  RGBDS-compatible symbol map), and `.lst` (the human-readable interleaved
  listing);
* **the build report family** owned by this stage:
  `placed_rom_plan.json`, `map.json`, `reachability_report.json`,
  `certs/reachability.cert.json`. Other reports
  (`build_manifest.json`, `provenance.json`, `compiler_feedback.json`,
  …) are owned by F-F1 and consume F-B15 outputs by hash.

After F-B15 closes, the only remaining Epic B work is F-B16
(FeasibilityRefinementLoop) and F-B17 (StageCache integration sweep).
F-B16 may, in a future chunk, ask F-B15 to **retry** under a different
`PlacementProfile` — but the retry is a re-invocation of the same stage,
not a refinement that mutates a previous PlacedRom. F-B15 itself is
deterministic and idempotent: same `SchedulePack` + same `TargetProfile`
+ same `PlacementProfile` ⇒ same byte sequence (§2.3, §10.4).

### 0.4 Cross-epic interactions

F-B15 sits at the intersection of five epics:

```text
Epic A → Epic B
  - gbf-foundation (BlobRef, BlobCodec, Hash256 wrappers)         consumed
  - gbf-store (StageCache) for K12/K13/K14/K15 cache wiring        consumed
  - gbf-asm types (AsmIR, Section, SectionRole, MachineEffect,
    PrivilegeClass, Builder, Encoder, Layout, Relax, RomBuilder)   consumed
  - gbf-hw (TargetProfile, region map, MBC5 invariants,
    cartridge_header, interrupt vectors, MemoryRegion classifier,
    PpuMode tables)                                                consumed
  - gbf-abi (BuildIdentityBlock, CompatibilityEnvelope,
    SemanticCheckpointSchema, HarnessCommandBlock layout,
    InferenceState prefix, FaultCode partition,
    InterruptPolicy, ResourceLease shape)                          consumed
  - gbf-runtime::banking (BankLease/BankGuard ABI; the only
    legal MBC-write path; codegen-emitted code MUST go through
    these helpers, never raw $2000/$3000/$4000/$0000 writes)       consumed
  - gbf-runtime nucleus (Bank0 sections from F-A5: boot,
    interrupts, scheduler, joypad, text, keyboard, video_commit,
    panic; reachability roots from these sections feed the
    whole-program analysis here)                                   consumed

Epic B (internal):
  - F-B2 / F-B4 (ResolvedCompilePolicy, ReportEnvelope rule,
    static-budget hash for cross-check)                            consumed
  - F-B13 (SchedulePack — input)                                   consumed
  - F-B14 (ScheduleCostReport — consumed for budget annotations
    in map.json)                                                   consumed
  - F-B16 (FeasibilityRefinementLoop — may request a retry under
    a different PlacementProfile; F-B15 is idempotent under
    fixed inputs, so retry is a re-invocation)                     downstream
  - F-B17 StageCache cross-cut                                     compatible

Epic C → Epic B (oracle correspondence):
  - F-C2 ArtifactOracle compares vs QuantGraph and GbInferIR;
    once the ROM exists, F-C3 ScheduleOracle compares the
    PlacedRom-resident ROM against an idealized GbSchedIR
    interpretation                                                 provided
  - F-C4 ConformanceEnvelope consumes
    certs/reachability.cert.json as a structural prerequisite      provided

Epic F → Epic B:
  - F-F1 gbf-report aggregates this stage's outputs into
    build_manifest.json + provenance.json + compiler_feedback.json consumes
  - F-F2 Certificates pins certs/reachability.cert.json shape and
    validates against an independent reference reachability
    walker in gbf-verify                                            consumes

Epic D → Epic B (deferred):
  - F-D1 (persistence) consumes the BuildIdentityBlock at the
    cartridge-header offset baked here                             provided
  - F-D2 (harness control plane) consumes the HarnessCommandBlock /
    HarnessResultBlock placements baked here                       provided
  - F-D3 (trace) consumes the trace ring placement baked here     provided
  - F-D5 (FaultPolicy) consumes the panic vector placement baked
    here                                                           provided
```

### 0.5 Milestone alignment

Per `planv0.md` §"Milestones," this chunk closes M2 and unblocks M3:

```text
M0    (DONE)  Foundation: Epic A infrastructure.
M0.5  (DONE)  F-B1 Compute Bringup: runtime/banking/harness/emulator
              proven for sustained integer compute. Merged: c2edbaa.

M1    (in progress)
              DenotationalOracle + ArtifactOracle + a single quantized
              dense kernel; first conformance.json; first CompileRequest
              wiring. F-B2/F-B3/F-B4/F-B5 land the entry, IRs, and
              static-budget filter.

M2    (target for THIS RFC)
              One shared micro-kernel resolved by RomWindowPlan; one
              expert payload bank; emulator diffing against
              ScheduleOracle; **first ReachabilityValidation pass
              integrated into the backend** (planv0 line 2914-ish).
              ↳ F-B15 (this chunk) closes the headline M2 deliverable.
                After F-B15 lands, M2 closure requires only F-C3
                (ScheduleOracle) and F-F2 (Certificates).

M3            Top-1 router, expert dispatch table, value/effect
              GbInferIR + ObservationPlan + RangePlan + StoragePlan
              wired end-to-end for a routed FFN under the cooperative
              scheduler.
              ↳ F-B15's PlacementProfile::PackedExperts surface is
                the M3 commitment for "multiple small experts may
                co-reside when legal."

M4+           Sequence-state block (BoundedKv first, then LinearState),
              SchedulePack mode switching, persistence, drift, fault
              recovery.
              ↳ F-B15 emits a SchedulePack-aware PlacedRom: each
                RuntimeMode keyed under SchedulePack.modes gets its own
                ResidencyEpoch-aligned section sequence. M4's mode
                switching is then a runtime decision, not a recompile.
```

F-B15 is therefore the **bridge from M2 to M3**. Without it, every
preceding stage's output is contract-frozen but never burned to bytes;
the runtime nucleus from F-A5 has no compiled inference body to host;
and F-C3/F-C4 cannot begin (ScheduleOracle wants something concrete to
diff against). After F-B15 lands, M2 closure is one PR away (F-F2 /
F-C3), and M3 has a deterministic placement substrate.

### 0.6 What the project as a whole gains when this chunk lands

```text
1. The pipeline produces a real ROM.
   F-B2..F-B14 leave the pipeline at "everything is typed and
   verified, no bytes written." F-B15 burns those decisions to bytes.
   Without F-B15, the rest of Epic B is contract-frozen at Stage 11.

2. ReachabilityValidation moves from declaration to proof.
   Earlier stages (F-A1's MachineEffect classifier; F-B13's
   ResourceStateValidation) declared rules. F-B15 *computes* the
   transitive closure and rejects the program when declarations
   disagree with computation (§9.6). This is the single defense
   against the "locks up after twenty minutes on cartridge" failure
   mode (planv0 line 1958, 1993).

3. PlacementProfile becomes a typed surface.
   StrictOnePerBank for bring-up, Budgeted for default, PackedExperts
   for tight builds. F-B16's RepairPolicy becomes plug-compatible
   with this enum: a profile fallback is a stage-12 retry, not a
   mid-stage mutation.

4. The build manifest becomes addressable.
   map.json + .sym + .lst + reachability_report.json +
   certs/reachability.cert.json are the load-bearing artifacts every
   downstream tool (gbf-debug, gbf-emu, gbf-report, gbf-verify, the
   harness) consumes by hash. F-B15 pins their schemas.

5. Determinism class becomes byte-stable.
   Same SchedulePack + same TargetProfile + same PlacementProfile +
   same runtime nucleus hash ⇒ same .gb byte sequence. This is the
   load-bearing invariant for cargo-test stability across CI runs.

6. Far-call legalization survives review.
   F-A1's PreLayoutOp::FarCall + F-A4's BankLease/BankGuard ABI is
   legalized inside PlacedRom (§10.2.2) — not as a runtime fixup,
   not as ad-hoc thunk insertion. Every cross-bank CALL becomes a
   typed thunk reachable from .sym and .lst.

7. The "AsmIR codegen" discipline is reusable.
   Codegen-from-SchedulePack here (§8) is the same shape every later
   IR-to-AsmIR transformer will use. Future kernels (F-H2) can
   author Builder calls in confidence that the codegen pipeline
   is shaped the same way for them.

8. The encoder stays tiny.
   Every judgment lives in PlacedRom. EncodedRom is a deterministic
   serializer (§11) with no policy and no choice points. This is
   the single-source-of-truth posture (constitution §VI.1) at the
   byte boundary.
```

### 0.7 Reading order for reviewers

A reviewer who has just read F-B11/F-B12 and is approaching this RFC for
the first time should read:

```text
§0    (this section) — placement and dependencies
§0a   TL;DR + closure conditions
§1    Project context — what F-B14 leaves; why all four sub-passes
      belong to one Feature; what this chunk is NOT
§2    Load-bearing decisions — the engineering choices that bracket
      the rest of the RFC
§5    Authority rules — what this RFC owns vs inherits
§6    Pipeline state machine — how Stage 12 plugs into Stages 11
      (input) and the build-report aggregation step (downstream)
§8    Sub-pass 1: AsmIR codegen
§9    Sub-pass 2: ReachabilityValidation (lattice + decision proc.)
§10   Sub-pass 3: PlacedRom (placement profiles, layout/legalization)
§11   Sub-pass 4: EncodedRom (.gb + .sym + .lst)
§12   StageCache algebra
§13   Diagnostic algebra (ASM-* / REACH-* / PLACE-* / ENC-* codes)
§14   Cross-stage interactions
§15   Task DAG
§16   Rejection classes
§17   Proof obligations
§18   End-to-end theorem
§19   Final concise contract
```

Skim §3, §4, §7, the spec pack appendix, and the `Amends planv0` log for
specifics.

## 0a. TL;DR

This chunk lands the **headline backend stage** of Epic B. It owns
**Stage 12** of the compiler pipeline, which contains four internal
sub-passes that ship as one Feature (`bd-18d`) but as a tightly-ordered
PR series:

* **Sub-pass 1 — `AsmIR` codegen.** Lower every `SchedSlice` from the
  frozen `SchedulePack` (F-B13's output) into a typed `Vec<Section>` of
  `gbf-asm` AsmIR. Every emitted op carries provenance back to its
  originating `SchedSlice`/`SchedOp`/`EffectId`. Compiler-generated code
  authors all MBC writes through F-A4's `BankLease`/`BankGuard` ABI.
  Vector stubs are emitted as **first-class layout entities** so the
  reachability pass and the listing/symbol-map can name them. Raw byte
  blobs are limited to what `gbf-asm`'s `Db`/`Dw` data directives can
  carry (provenance-bearing) plus the cartridge header bytes assembled
  by F-A1's ROM builder; there is **no `Raw(Vec<u8>)` escape hatch**
  (per F-A1 RFC's stronger Rule 10 — "no escape hatch at all").

  F-B15 owns the codegen pipeline. **AsmIR types are owned by F-A1; this
  RFC consumes them unchanged.** §8.7 pins the boundary.

* **Sub-pass 2 — `ReachabilityValidation`.** Compute (not trust) the
  transitive reachability classes of code/data after the call/branch
  /thunk edge graph is fully legalized. Six classes: `IsrReachable`,
  `YieldResumeReachable`, `FaultPathReachable`, `HarnessEntryReachable`,
  `BankLeaseProtected`, `NormalOnly` (§9.1 lattice). Validate seven hard
  rules (`planv0.md` line 1934+; §9.3). Where the computation disagrees
  with F-B13's `ResourceStateValidation` annotations, **F-B15 wins** and
  emits diagnostics that the annotations were wrong (§9.6). Outputs:
  `reachability_report.json` + `certs/reachability.cert.json`.

* **Sub-pass 3 — `PlacedRom`.** Run layout + branch relaxation +
  far-call thunk insertion + bank-switch coalescing + deterministic
  section ordering + stable symbol naming + common-bank vs expert-bank
  partitioning under the resolved `PlacementProfile` (`StrictOnePerBank`
  / `Budgeted` / `PackedExperts`). Enforce six global constraints
  (`planv0.md` line 1966+; §10.5): no section crosses a bank boundary,
  all relative branches in range or rewritten, all expert sections
  satisfy residency rules, all SRAM/WRAM arenas fit, all continuation
  targets valid + reachable, bank packing is deterministic. Outputs:
  `placed_rom_plan.json` + `map.json`.

  **The ISR residency rule is enforced by `PlacedRom` against
  `ReachabilityValidation`'s computed classification, not against
  declaration alone** (§10.2.5). This is the load-bearing safety
  property of the entire chunk.

* **Sub-pass 4 — `EncodedRom`.** Emit bytes only. `.gb`, `.sym`, `.lst`.
  The encoder is intentionally tiny: every byte traces to a `PlacedRom`
  decision, every symbol traces to a `Section`, every listing line
  traces to a `Section`+offset. There are no choice points and no
  policies in this sub-pass.

These four sub-passes ship as one Feature because they share the
**back-end-of-pipeline** shape: AsmIR codegen produces sections that
ReachabilityValidation reasons about, PlacedRom places those sections
with the reachability-derived residency facts as constraints, and
EncodedRom serializes the placed bytes. They share a diagnostic
envelope, JSON canonicalization rule, self-hash convention,
`StageCache` key construction, and the F-A1 typed encoder boundary.
Splitting into separate Features would break the typed seams (§1.4).

The chunk closes only when:

1. AsmIR codegen is a deterministic pure function of `SchedulePack`,
   `ResolvedCompilePolicy`, `TargetProfile`, the F-A5 runtime nucleus
   sections (by hash), and the F-A1 cartridge header (by hash); and is
   byte-identical across two consecutive regenerations on a clean
   checkout.
2. ReachabilityValidation rejects every malformed-input class enumerated
   in §9.3 and every privilege-violation class enumerated in §13.2;
   classifies every byte of code/data into exactly one class from the
   six-class lattice; emits `reachability_report.json` and
   `certs/reachability.cert.json` whose `report_self_hash` round-trips
   through the F-B2/F-B4 self-hash convention.
3. PlacedRom places every section without violating any of the six
   global constraints (§10.5); produces a deterministic byte ordering
   under each of the three `PlacementProfile` variants; populates
   `placed_rom_plan.json` and `map.json` whose canonical-JSON
   self-hashes round-trip.
4. EncodedRom produces a `.gb` whose Pan-Docs cartridge-header
   checksums match (delegated to F-A1's `gbf-asm::rom`); whose `.sym`
   round-trips through the agreed Game Boy symbol format
   (`BB:AAAA name` per line, sorted); whose `.lst` round-trips through
   a structural validator that names every byte's source section +
   offset.
5. `StageCache` keys K12 (AsmIR codegen), K13 (ReachabilityValidation),
   K14 (PlacedRom), K15 (EncodedRom) are pinned and tested.
6. F-A4's `mbc_write_provenance_audit` (already in tree) walks the
   F-B15-emitted sections and asserts every `MachineEffect::
   StoreToMbcRegister` originated in `gbf-runtime::banking` (no raw
   MBC writes from generated code).
7. F-B16's `RepairPolicy` is named-only — F-B15 keeps `PlacementProfile`
   pluggable but does not implement profile fallback. F-B16, when it
   lands, drives F-B15 retries; F-B15 itself never retries.
8. The `BuildIdentityBlock` (F-A3) is emitted into the cartridge header
   region (§3A.3 of F-A5; §11.1 here) with all four lineage hashes
   populated: build hash, artifact-core hash (from F-B2),
   runtime-nucleus hash (from F-A5), compile-request hash (from F-B2).

The chunk does **not** include:

* The runtime itself — owned by F-A5 (Bank0 nucleus); F-B15 consumes the
  emitted sections.
* The assembler primitives — owned by F-A1 (`gbf-asm::isa`,
  `gbf-asm::section`, `gbf-asm::encoder`, `gbf-asm::layout`,
  `gbf-asm::relax`, `gbf-asm::rom`); F-B15 consumes the typed eDSL,
  encoder, and layout/relax engine unchanged.
* The loader / OS / boot ROM — DMG hardware does its own boot before
  the cartridge entry stub. F-B15 emits the cartridge header and
  entry stub through F-A1's `gbf-asm::rom` builder; the boot ROM
  itself is not modeled.
* `RepairProposal` evaluation — owned by F-B16. F-B15 emits diagnostics
  classifying the failure mode (§13) but does not synthesize repairs.
* `RuntimeChromeBudget` re-validation — F-B4 already ran static budget
  projection; F-B15 trusts the resolved budget by hash and only
  rejects actual placement infeasibility (§13.3, §16.4).
* `KernelRegistry` definition — kernels are owned by F-H1/F-H2; F-B15
  consumes their AsmIR sections by KernelId+ResidencyClass (§8.1.6).
* The reference reachability walker for the certificate cross-check —
  owned by `gbf-verify` (F-F2 territory); F-B15 emits the certificate,
  `gbf-verify` independently re-validates it.
* `ScheduleOracle` integration — owned by F-C3; F-B15 produces the
  byte-level artifact F-C3 will diff against, but does not implement
  the diff.

## 1. Project context — where this stage sits in the milestone sequence

### 1.1 What F-B2..F-B14 leave on the table

Per the prior RFCs, by the time this chunk begins, the following hold:

* `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`,
  `TargetDataLoweringArtifact`, calibration, hint bundle, and
  `CompileRequest` are admissible, hash-bound, and traceable through
  `artifact_validation.json` and `policy_resolution.json` (F-B2/F-B4).
* `ResolvedCompilePolicy` is the single answer to "what policy governed
  this build," with provenance for every load-bearing scalar
  (`PolicyProvenance::TargetDefault | ProfileDefault |
  CompileRequestOverride | HintBundle | Calibration`). F-B16 will later
  introduce `RepairProposal`/`AuthorizedRelaxation` provenance; F-B15
  consumes only the existing five.
* `RuntimeChromeBudget` is honored at the static byte-math level. F-B4
  has already emitted `static_budget.json`. F-B15 trusts that envelope
  by hash; if PlacedRom finds a hard fit failure that contradicts the
  static envelope, that is a `PLACE-CHROME-DRIFT` diagnostic
  (§13.3.4), not a silent re-derivation.
* `QuantGraph` (F-B3) and `GbInferIR` (F-B5) exist with full canonical
  reference semantics. F-B15 does not consume `QuantGraph` directly; it
  consumes `GbInferIR` ids only as provenance roots reachable through
  `SchedulePack.modes[mode].slices[s].ops[o].provenance.infer_op`.
* `ObservationPlan` (F-B6) has attached `SemanticCheckpointId` and
  `TraceProbeId` references. The compact ids minted by `gbf-codegen`'s
  checkpoint/probe registries are part of `SchedSlice.ops` provenance.
* `RangePlan` (F-B7) has produced `range.cert.json`. F-B15 trusts this
  by hash; accumulator-overflow safety is a Stage-7 property and is not
  re-checked here.
* `StoragePlan` (F-B8) has bound every `ValueId` to a
  `StorageBinding { class, lifetime, materialization, alias_class }`.
  F-B15 consumes those bindings transitively through `SchedulePack`'s
  `live_wram` / `live_sram` arena slot lists.
* `SramPagePlan` (F-B9) has typed every persistent page with a
  `PersistPageId` + `CommitGroupId`. F-B15 reserves the SRAM byte
  ranges per F-B12 `ArenaPlan` and emits `arena.cert.json` cross-refs
  in the placed-rom report.
* `RomWindowPlan` (F-B10) has resolved every kernel's residency choice
  (`KernelResidency::Bank0Fixed | CommonBank(BankId) |
  ExpertBank(ExpertId) | WramOverlay(OverlayId)`). F-B15's PlacedRom
  enforces these residency choices as hard placement constraints
  (§10.2.5).
* `OverlayPlan` (F-B11) has reserved WRAM-overlay regions and scheduled
  install events. F-B15's AsmIR codegen emits the install-event
  trampolines and the PlacedRom honors the overlay-region byte budget.
* `ArenaPlan` (F-B12) has assigned concrete byte ranges in WRAM hot
  arena, WRAM overlay, HRAM fast flags, and SRAM persistent pages.
  F-B15 emits these as data-section directives where relevant
  (`Db`/`Dw` for ROM-resident constants; arena reservations otherwise
  travel as memory-map entries in `map.json`).
* `GbSchedIR` (F-B13) has been frozen into a `SchedulePack` keyed by
  `RuntimeMode`. Every `SchedSlice` has `id`, `ops`, `hard_cycles_to_safe_point`,
  `soft_target_cycles`, `max_interrupt_latency`, `resources`,
  `live_wram`, `live_sram`, `yield_kind`, `yield_check`, `entry_residency`,
  `interrupt_policy`, `required_leases`, `exit_kind`. Every
  `ResidencyEpoch` has a `rom_window` + optional `overlay` + slice
  list. `ResourceStateValidation` (Stage 10.5) has produced
  `certs/resource_state.cert.json` proving lease balance, no illegal
  yield across non-resumable leases, no ISR-visible dependency on
  leased switchable state, and overlay-shadow assumptions consistent
  with declared residency.
* `ScheduleCostAnalysis` (F-B14) has produced `schedule_cost.json` with
  per-mode `EstimatedCostDelta`s. F-B15 consumes this for budget
  annotations baked into `map.json` (§10.8.4) but does not re-derive
  costs.

This chunk is responsible for **burning all of those frozen decisions
into bytes** and for **proving** that the resulting byte sequence is
safe to deploy to a real DMG cartridge under the hard rules
`planv0.md` makes about ISR residency, banking discipline, continuation
correctness, and bank-switch atomicity.

### 1.2 What M2/M3 commits to and how this chunk delivers it

Per `planv0.md` §"Milestones":

> **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus one
> expert payload bank, with exact emulator diffing against
> `ScheduleOracle` and checkpoint alignment against `ArtifactOracle` at
> `SemanticCheckpointId` boundaries; **first `ReachabilityValidation`
> pass integrated into the backend**.
> **M3**: top-1 router, expert dispatch table, value/effect `GbInferIR`
> + `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for
> a routed FFN under the cooperative scheduler.
> **M4+**: sequence-state block, `SchedulePack` mode switching,
> persistence, drift, fault recovery.

Mapping onto F-B15's deliverables:

* M2 commitment "first `ReachabilityValidation` pass integrated into
  the backend" requires the §9 sub-pass.
* M2 commitment "shared micro-kernel resolved by `RomWindowPlan`,
  expert payload bank" requires §10's `PlacementProfile::Budgeted`
  with common-bank vs expert-bank partitioning. The shared
  micro-kernel sits in a `SectionRole::CommonKernel`-tagged section;
  the expert payload sits in a `SectionRole::ExpertPayload(ExpertId)`-
  tagged section.
* M2 commitment "exact emulator diffing against `ScheduleOracle`"
  requires the encoded `.gb` — F-B15's §11 deliverable — plus the
  `.sym` map so `gbf-debug` can resolve checkpoint addresses.
* M3 commitment "top-1 router, expert dispatch table" requires §10's
  `PlacementProfile::PackedExperts` to land tight expert builds. F-B15
  ships the profile as a typed selector; the routing-table-aware
  cost estimation lives upstream in F-B14 / F-B10.
* M4+ commitment "`SchedulePack` mode switching" requires F-B15 to
  emit per-`RuntimeMode` sections under each `ResidencyEpoch`. F-B15
  ships the data-shape; the mode-switch policy at runtime is owned
  by F-D5 / F-A5's scheduler.

Because M2 lands before M3/M4, this chunk's headline target is the
M2 deliverables — a `Budgeted` placement profile producing one shared
common-bank kernel and one expert payload bank, with the
`ReachabilityValidation` certificate emitted, and the encoded `.gb`
diffable against `ScheduleOracle` once F-C3 lands.

### 1.3 What this chunk retires for the rest of Epic B

By the time F-B16 (FeasibilityRefinementLoop) and F-B17 (StageCache
sweep) chunks begin:

* The compiler can produce a real ROM. F-B16's refinement loop has a
  concrete artifact to retry against rather than an abstract "pretend
  Stage 12 ran" placeholder.
* The reachability cert exists. F-F2's certificate tooling has a real
  certificate to validate, and the independent reachability walker in
  `gbf-verify` has a concrete byte-level artifact to walk.
* The `map.json` schema is stable. `gbf-debug` (F-A8), `gbf-emu`
  (F-A7), `gbf-bench` (Epic E), and the harness (F-D2) all consume
  `map.json` by hash and depend on its schema being pinned here.
* The `.sym` and `.lst` shapes match `gbf-debug`'s session-file
  expectations. The agent-debugger workflow (CLAUDE.md §"Project
  Skills" → `gbf-debug-usage`) is plug-compatible with the symbol
  output emitted here.
* `SchedulePack`-keyed multi-mode emission is a structural fact.
  Future mode-switch work (M4+) plugs into the per-`RuntimeMode`
  section sequence; it does not need to extend Stage 12's contract.
* The `BuildIdentityBlock` is at a known cartridge offset. The
  identity handshake (per F-A3 §3.1) becomes operationally true.

This chunk's job is to retire the **byte-emission**, **whole-program
proof**, **placement determinism**, and **identity** preconditions of
the rest of the toolchain. It is the last shift-left filter inside
the compiler proper; downstream tooling (oracles, harness,
emulator/debugger, certificates) inherits the typed byte boundary
that ships here.

### 1.4 Why this is one Feature, not four (or two)

The natural unit is "the back-end of the pipeline": IR-to-AsmIR
codegen, whole-program proof, placement, encoding. Splitting the
sub-passes into separate Features was considered and rejected:

* **One feature per sub-pass (four Features)** — would split on
  pipeline order. Each split breaks a typed seam:
  * AsmIR-codegen ↔ ReachabilityValidation: the reachability pass
    consumes the codegen's output edge graph by `(SectionId, OpIndex)`
    references that are *not yet stable* unless the codegen's typed
    output discipline is co-developed with the reachability pass's
    consumption discipline. If they ship in separate PRs, the
    reachability pass's input shape will drift across the PR boundary.
  * ReachabilityValidation ↔ PlacedRom: the "ISR residency rule is
    proven, not declared" property (`planv0.md` line 1944) can only be
    enforced if PlacedRom rejects assignments that the reachability
    pass disagrees with. If the two passes ship in separate PRs, the
    enforcement contract has no PR home.
  * PlacedRom ↔ EncodedRom: the encoder's "tiny" property
    (`planv0.md` line 1983) requires the placement pass to have made
    every choice already. If the encoder ships in a separate PR, the
    natural drift is encoder choice points (e.g. "encoder picks the
    final padding") leaking into the encoder.
  Four Features also fragments PR review: each would carry only ~1/4
  of the load-bearing contract surface and force reviewers to
  context-switch four times.

* **Two features (split AsmIR + Reachability vs. PlacedRom + EncodedRom)
  ** — natural seam is "before placement vs. placement." But the
  placement pass enforces the residency rule that the reachability
  pass *computes*, and the encoder consumes the placement decisions
  the placement pass *makes*. Both natural seams cross the proposed
  split. Two-Feature split would still force the residency-rule
  enforcement to span two PRs.

* **Three features (split EncodedRom out alone)** — the encoder is
  intentionally tiny (§11.4). Putting it in its own Feature inverts
  the size/PR-review tradeoff: the smallest sub-pass would carry the
  loudest review burden because reviewers would need to re-establish
  the PlacedRom→EncodedRom contract from scratch. Better to ship the
  encoder inside the same Feature as PlacedRom, where the contract is
  one-PR-distant.

* **One Feature (this RFC's choice)** — all four sub-passes ship as
  one bead (`bd-18d`) but as a strictly-ordered task series (§15) with
  one task group per sub-pass. Each task group is its own PR; the
  sub-pass typed seams are co-developed within the bead. The bead
  closes only when all four sub-passes meet the closure conditions in
  §0a. PR review fragments by sub-pass (PR-per-task-group), not by
  Feature.

The bead-level atomicity matters because **Stage 12 is the natural
unit `gbf-codegen` calls**: there is one `run_stage12` driver in
`gbf-codegen::backend` that internally orchestrates the four
sub-passes; downstream consumers (build-report aggregation, F-B16's
refinement loop) do not see the four sub-passes as separate stages.
Treating Stage 12 as one Feature aligns the Feature granularity with
the consumption granularity. Per F-B3/F-B5 §1.4's discussion of
similar choices: "the natural seam is two Features" was the right
call there because each canonical IR has a separate downstream
consumer. Here the natural seam is one Feature because there is one
consumer.

### 1.5 What this chunk is NOT

The chunk is **medium in scope** but **very large in contract surface**.
To prevent scope creep, here is what this chunk explicitly is not:

* It is **not** the assembler primitives. F-A1 owns `Instr`,
  `Section`, `SectionRole`, `MachineEffect`, `PrivilegeClass`,
  `Builder`, `Encoder`, the layout pass, the relax/legalization
  fixed-point, and the ROM builder. F-B15 consumes those types and
  invokes those passes; it does not redefine them.
* It is **not** the `BankLease`/`BankGuard` ABI. F-A4 owns the ABI;
  F-B15 generates calls to the F-A4 helpers from compiled code. Raw
  MBC writes from generated code are forbidden (§2.1; cited
  `planv0.md` line 1921).
* It is **not** the Bank0 nucleus. F-A5 owns boot, interrupts,
  scheduler, joypad, text, keyboard, video_commit, panic. F-B15
  consumes the runtime nucleus by hash and emits the inference body
  that the nucleus dispatches into.
* It is **not** the runtime drift monitor, the fault-policy recovery
  exerciser, or the safe-mode trigger evaluator. F-D4 / F-D5 own
  those; F-B15 emits the byte-level support (panic vector
  placement, `FaultCode`-discriminated handlers) but no policy.
* It is **not** the loader / boot ROM. DMG hardware boots its own
  internal boot ROM; F-B15 emits a Pan-Docs-conformant cartridge
  header and the entry stub at `$0100`, both via F-A1's `gbf-asm::rom`
  builder.
* It is **not** an artifact migration tool. `gbf-migrate` is deferred
  to F-A6b; F-B15 only refuses to compile when the
  `ResolvedCompilePolicy` says the inputs are inadmissible (a
  precondition F-B2 already enforces).
* It is **not** a kernel implementer. F-H1 (`KernelSpec`) and F-H2
  (kernel implementations) own the kernels. F-B15 places kernels
  according to F-B10's `RomWindowPlan` and emits their entry stubs
  in expert banks per F-B11/F-B12's reservations.
* It is **not** the report aggregator. F-F1 (`gbf-report`) aggregates
  build reports into `build_manifest.json`, `provenance.json`,
  `compiler_feedback.json`. F-B15 emits four specific reports
  (`placed_rom_plan.json`, `map.json`, `reachability_report.json`,
  `certs/reachability.cert.json`); F-F1 reads those by hash.
* It is **not** the certificate cross-validator. F-F2 +
  `gbf-verify` independently re-walks the reachability graph to
  validate `certs/reachability.cert.json`. F-B15 emits the
  certificate; the cross-validation is downstream.
* It is **not** a refinement loop. F-B16 owns `RepairProposal`,
  `ConstraintDelta`, the loop driver, and `repair_report.json`. F-B15
  is idempotent under fixed inputs; F-B16 drives retries by varying
  the inputs (e.g. `PlacementProfile`).
* It is **not** the cycle-cost producer. F-B14 owns the cost envelope
  per mode. F-B15 consumes `schedule_cost.json` for budget
  annotations baked into `map.json`; the costs are not re-derived
  here.
* It does **not** assume any concrete model topology. Tests use
  synthetic `SchedulePack` fixtures; the chunk's structural
  correctness is independent of "is this a dense kernel or a routed
  FFN." Per `planv0.md` §"Reports and artifacts," `map.json`,
  `.sym`, `.lst`, `reachability_report.json`, and
  `certs/reachability.cert.json` are first-class regressions
  regardless of the inference workload that produced them.
* It is **not** an oracle. F-C2/F-C3 own `ArtifactOracle` /
  `ScheduleOracle`. F-B15 produces the byte-level artifact that
  `ScheduleOracle` (once F-C3 lands) will diff against; the oracle
  itself is not here.

### 1.6 Relationship to F-A1's `gbf-asm` (the AsmIR owner)

F-A1 (`bd-ssm`) owns the AsmIR types — every `Instr` variant, every
`Section`/`SectionRole`/`MachineEffect`/`PrivilegeClass`, the
`Builder`, the encoder, the layout/relax fixed-point, the ROM
builder, `.sym`/`.lst` emitters. F-B15 consumes all of these
unchanged.

The boundary is crisp:

* F-A1 owns the **typed authoring API**: how an `Instr` is shaped,
  how a `Section` carries items, how `Builder` validates emissions,
  how the encoder turns `Instr`s into bytes, how layout assigns
  sections to banks, how relax legalizes branches and inserts thunks.
* F-B15 owns the **codegen pipeline**: how a frozen `SchedulePack`
  becomes a `Vec<Section>`, how those sections compose with the F-A5
  runtime nucleus sections, how the whole-program reachability proof
  runs over the assembled section graph, how `PlacementProfile`
  drives placement decisions, how the resulting `PlacedRom` is
  serialized.

F-B15 does not modify any F-A1 type. If the codegen pipeline
discovers a missing primitive in `gbf-asm` (e.g. a new
`PreLayoutOp` variant for a banking pattern), the addition lands in
F-A1's type system via a follow-up bead and a `gbf-asm` PR; F-B15
then consumes the new variant. This RFC does **not** reserve any
new `Instr`/`Section`/`SectionRole`/`MachineEffect`/`PrivilegeClass`
variant — every primitive used by codegen is already in F-A1's
shipped surface (commits `ec10b45`, `53d1d82`, `7a5c687` per F-A5
RFC §0.0.5).

The reverse is not true: F-A1 cannot consume F-B15 types because
`gbf-codegen` depends on `gbf-asm`, not the other way around. The
dependency direction is one-way.

### 1.7 Relationship to F-A4's `BankLease`/`BankGuard` ABI

F-A4 (`bd-1sv`) owns the only legal MBC-write path (`planv0.md`
line 1921; F-A4 RFC §0). The shipped surface (per F-A5 RFC §0.0.6)
includes:

* `lease_rom_switchable(b, spec) -> BankGuard` for ROM-bank acquire;
* `lease_sram(b, spec) -> BankGuard` for SRAM-bank acquire;
* `release_bank(b, guard, return_state)` for release;
* `lower_banking_shadow_zero_init(b)` for boot zero-init (called by
  F-A5);
* HRAM banking-shadow constants (`$FF80..=$FF83`);
* `InterruptSafetyKind` + `InterruptSafetyTable` declaration
  substrate for ISR-residency annotation;
* `BankingPreLayoutLowering` — the production lowering for
  `PreLayoutOp::BankLease` / `BankRelease` / `AssertBank`;
* `mbc_write_provenance_audit` — walks emitted bytes and asserts
  every `MachineEffect::StoreToMbcRegister` originated in
  `gbf-runtime::banking`.

F-B15's AsmIR codegen generates calls to the F-A4 user-facing API
(`lease_rom_switchable`, `lease_sram`, `release_bank`) for every
cross-bank operand load required by a compiled slice. The lowering
of those calls — emitting the actual `Instr` sequence that performs
the MBC5 register write, updates the HRAM shadow, and brackets the
write in a short critical section — happens inside F-A4's
`BankingPreLayoutLowering` during the F-A1 layout/relax pass, which
F-B15 invokes (§10.2).

The audit (`mbc_write_provenance_audit`) runs against F-B15-emitted
bytes as one of F-B15's closure gates (§0a item 6). If any
`MachineEffect::StoreToMbcRegister` in an F-B15-emitted section has
provenance pointing outside `gbf-runtime::banking`, the build fails
with `ASM-MBC-PROVENANCE-VIOLATION` (§13.1.4).

`KeepCurrentProof` and `LeaseLifetime::ResumeWindow` /
`LeaseLifetime::Token` are F-A4-deferred surfaces (per F-A5 RFC
§0.0.6 item 6). F-B15 does not need either in M2 closure scope:
the M2 deliverable is a single-mode Budgeted placement with one
common-bank kernel and one expert-bank payload, with no
cross-slice resumption that would require `ResumeWindow`. M3+ work
that resumes inference slices owning a borrowed bank will require
both surfaces; that is out of `bd-18d` closure scope.

### 1.8 Relationship to F-A5's Bank0 nucleus

F-A5 (`bd-2r1`) owns the Bank0 nucleus sections: boot, interrupts,
scheduler, joypad, text, keyboard, video_commit, panic. F-B15
consumes those sections by hash and composes them with the
codegen-emitted inference sections into the assembled
`Vec<Section>` that PlacedRom places.

The composition shape:

```text
Vec<Section> ::=
    [F-A5 nucleus sections]      // boot, interrupts, scheduler,
                                 // joypad, text, keyboard,
                                 // video_commit, panic, isr_stubs,
                                 // far-call thunks (Bank0-resident
                                 // by F-A4's check_lease_emission_legal)
  ++ [F-B15 codegen sections]    // per-slice AsmIR, per-mode
                                 // continuation entry, per-expert
                                 // entry stubs, common-bank kernel
                                 // bodies, expert-bank tensor
                                 // payloads (as Db/Dw directives)
```

The `runtime_nucleus_hash` (per F-A5 §3H.3) is computed over a
*normalized* Bank 0 nucleus image with `BuildIdentityBlock`
linker-filled hash fields zeroed. F-B15 reads
`runtime_nucleus_hash` from the F-A5-emitted nucleus and pins it
into the `BuildIdentityBlock` (§11.1.4), then writes the
`BuildIdentityBlock` into the cartridge header region.

The `RuntimeShellModule` enum (F-A5 §0.0.5) is consumed by F-B4's
`RuntimeChromeBudget`; F-B15 does not directly consume it, but the
PlacedRom emitter must respect any `FutureReservation`-class byte
ranges in Bank 0 that F-A5 declared (§10.5.7).

The reachability roots (per `planv0.md` line 1995) are five-class:
`$0040` VBlank, `$0048` LCD STAT, `$0050` Timer, `$0058` Serial,
`$0060` Joypad. F-A5 emits the vector stubs as first-class layout
entities; F-B15's reachability pass enumerates them as the
`IsrReachable` class roots (§9.2.2). The follow-up bead `bd-3s0s`
("T-A1x.2: Make interrupt vectors `ReachabilityValidation` roots")
is the explicit dependency edge: it is `blocks` on `bd-18d` and
must close before F-B15 ships.

### 1.9 Relationship to F-B13 (input) and F-B14 (cost annotations)

F-B13 (`bd-9ae`, `GbSchedIR + ResourceStateValidation`, Stages
10/10.5) is F-B15's direct input. The handoff is:

```text
F-B13 produces:
  SchedulePack {
    modes: BTreeMap<RuntimeMode, GbSchedIR>,
    epochs: BTreeMap<RuntimeMode, Vec<ResidencyEpoch>>,
    checkpoint_schema_hash: Hash256,
    switch_policy: ModeSwitchPolicy,
  }
  certs/resource_state.cert.json (proves lease balance + ISR-state
                                  + overlay-shadow consistency)

F-B15 consumes:
  SchedulePack by hash (frozen — no mutation in this stage).
  certs/resource_state.cert.json by hash (cross-check input — F-B15's
    ReachabilityValidation §9.6 checks that its computed classes
    agree with F-B13's annotations; disagreement is a hard
    diagnostic with F-B15's classes winning).
```

F-B14 (`ScheduleCostAnalysis`, Stage 11) produces
`schedule_cost.json` with per-mode `EstimatedCostDelta` envelopes.
F-B15 consumes this for budget annotations baked into `map.json`
(§10.8.4 fields: `expected_cycles_per_token`, `bank_switches_per_token`,
`worst_case_interrupt_latency`). F-B15 does **not** re-derive
costs; if the static placement reveals a layout that violates the
F-B14 envelope, that is a `PLACE-COST-DRIFT` diagnostic
(§13.3.5) inviting F-B16 to re-run F-B14 with the revised layout —
not a silent re-derivation here.

### 1.10 Why this chunk has no oracle handshake

Unlike F-B3/F-B5 (which is an oracle-correspondence point), F-B15
does not produce an oracle-comparable IR. The byte-level artifact
F-B15 emits is what `ScheduleOracle` (F-C3) eventually diffs against
the runtime emulator — but the diff is downstream of this RFC, owned
by F-C3.

The reason F-B15 is not itself an oracle handshake: the four
sub-passes are *operational* transforms (codegen, proof, placement,
encoding). They do not introduce new semantics — every value's
meaning is fixed by `GbInferIR` (F-B5) and every operation's
schedule is fixed by `GbSchedIR` (F-B13). F-B15 *implements* those
fixed semantics on the LR35902 target; it does not re-derive them.

This is also why F-B15 does not emit `conformance.json`:
conformance is denotational-vs-artifact, owned by F-C2; F-B15's
output is one input to F-C3's eventual schedule-vs-emulator
comparison, not the comparison itself.

## 2. Load-bearing decisions

The seven decisions in this section are the engineering choices that
bracket the rest of the RFC. They are listed in the order a reviewer
should re-derive them when reading the technical sections.

### 2.1 All compiler-generated code goes through `BankLease`/`BankGuard`

**Decision**: F-B15-emitted code MUST author every MBC5-register
write through F-A4's `BankLease`/`BankGuard` ABI. Raw MBC writes
(`LD ($0000), A`, `LD ($2000), A`, `LD ($3000), A`, `LD ($4000), A`)
in compiler-generated sections are a **hard reject**.

**Cite**: `planv0.md` line 1921 ("compiler-generated code may **not**
emit raw MBC writes directly; it must go through the
`BankLease`/`BankGuard` ABI in `gbf-runtime::banking`"). F-A4 RFC §0
("the four MBC-writing helpers are the only audited lowering path
for MBC5 writes").

**Mechanism**:

1. AsmIR codegen (§8) emits `PreLayoutOp::BankLease`,
   `PreLayoutOp::BankRelease`, and `LegalizationOp::FarCall` —
   never raw `Instr::LdDirectFromA { addr: 0x2000, .. }` or
   equivalents.
2. F-A1's layout/relax pass invokes F-A4's
   `BankingPreLayoutLowering` to lower those pseudo-ops into
   `Instr` sequences inside `gbf-runtime::banking`. The lowered
   `Instr`s carry provenance pointing into `gbf-runtime::banking`.
3. F-A4's `mbc_write_provenance_audit` runs as one of F-B15's
   closure gates (§0a item 6) and asserts every emitted
   `MachineEffect::StoreToMbcRegister` has provenance from
   `gbf-runtime::banking`.
4. The AsmIR codegen's `Builder` emission validates `PrivilegeClass`
   — F-A1's existing `Builder::validate_effect` already rejects
   `StoreToMbcRegister` from `Normal`-class sections, which is
   what compiler-generated sections are.

The closure gate is structural: a malformed codegen path that tried
to emit a raw MBC write would fail `Builder::validate_effect` at
emission time, well before reaching the audit. The audit is a
belt-and-suspenders check for the case where a future codegen path
(or a future audited escape hatch) lowers an `Instr` directly.

**Rejection class**: `ASM-MBC-RAW-WRITE` (§13.1.3),
`ASM-MBC-PROVENANCE-VIOLATION` (§13.1.4).

### 2.2 Vectors are first-class layout entities

**Decision**: Interrupt vectors at `$0040`, `$0048`, `$0050`,
`$0058`, `$0060` are **typed sections owned by F-A5** that F-B15's
PlacedRom places at fixed addresses, names in `.sym`, lists in
`.lst`, and registers as reachability roots in
`reachability_report.json`. Post-assembly vector byte mutation is
forbidden in F-B15-emitted ROMs.

**Cite**: `planv0.md` lines 1995–2003 ("`gbf-asm` owns named vector
slots such as `$0040` VBlank, emits their stubs through typed
instructions, reserves the bytes during layout, and exposes them as
reachability roots. Post-assembly vector byte mutation is allowed
only as a legacy bringup adapter for packets such as F-B1;
production backends should place vectors first-class"). F-A5 RFC
§3.3 (vector stubs annotated `PrivilegeClass::InterruptHandler` +
`ExecutionContext::InterruptHandler`).

**Mechanism**:

1. F-A5 emits the five vector stubs as `Section`s with
   `SectionRole::IsrReachable` and `PrivilegeClass::InterruptHandler`.
   The bead `bd-3s0s` ("T-A1x.2: Make interrupt vectors
   `ReachabilityValidation` roots") tracks the F-A1 layout work to
   place these as fixed-address sections.
2. F-B15's AsmIR codegen does **not** synthesize vector sections.
   It consumes the F-A5-emitted vector sections by hash.
3. F-B15's PlacedRom places the vectors at the Pan-Docs-defined
   absolute addresses (`gbf_hw::interrupts::INT_VECTOR_*`) via F-A1's
   layout pass with the `LayoutPlan::pin_vector` API.
4. F-B15's ReachabilityValidation enumerates the five vector
   addresses as the `IsrReachable` class roots (§9.2.2).
5. F-B15's EncodedRom serializes the vectors first (the cartridge
   header gets `$0100..=$014F`; vectors at `$0040..=$0067` precede
   it numerically).

Legacy-bringup adapters (e.g. F-B1's runtime that mutated vectors
post-assembly) are explicitly out of scope for production builds:
F-B15-emitted ROMs reach a fully legalized vector placement before
any byte is written.

**Rejection class**: `PLACE-VECTOR-NOT-FIRST-CLASS` (§13.3.6),
`REACH-VECTOR-NOT-ROOT` (§13.2.6).

### 2.3 Reachability rules are computed, not declared

**Decision**: The seven validation rules in `planv0.md` line 1934+
(ISR residency, no forbidden MBC writes on privileged paths, no
illegal `MachineEffect` on privilege-restricted paths, no
switchable-bank dependency on ISR or resume paths, no illegal
reentrancy through bank guards, no unreachable continuation
targets, no fault path on non-resident data) are validated by
**computing the transitive reachability classes** of every byte of
code/data after the call/branch/thunk edge graph is fully
legalized. F-B15 does not trust earlier-stage annotations as proof;
it uses them as **hypotheses** that the computed classes either
confirm or refute.

**Cite**: `planv0.md` line 1944 ("This pass **computes, rather than
trusts**, which code/data must be Bank0/HRAM/fixed-WRAM only and
which paths may legally depend on switchable residency").
`planv0.md` line 1993 ("computed by `ReachabilityValidation`, not
declared and hoped for").

**Mechanism**:

1. F-B13's `ResourceStateValidation` and F-A1's `MachineEffect`
   classifier both attach **declared** annotations to
   sections/ops/leases.
2. F-B15's ReachabilityValidation (§9) walks the call/branch/thunk
   graph from a fixed set of roots (vectors, harness entry,
   continuation entry per mode, fault entry) and computes the
   transitive class assignment per the lattice in §9.1.
3. Where the computed class disagrees with the declared annotation,
   F-B15 **rejects the build** and emits a
   `REACH-CLASS-DISAGREEMENT` diagnostic (§13.2.7) naming both the
   declared and computed classes plus the path that produced the
   disagreement.
4. The certificate `certs/reachability.cert.json` records the
   computed class per `(SectionId, Offset)` pair; downstream
   consumers (F-F2, `gbf-verify`) re-run the computation on the
   encoded bytes for an independent cross-check.

This is the load-bearing safety property of the entire chunk: a
declaration mismatch is exactly the failure mode that "locks up
the cartridge after twenty minutes" (`planv0.md` line 1993). The
computational proof is the only correct posture.

**Rejection class**: `REACH-*` family (§13.2).

### 2.4 Placement profiles are explicit, not implicit

**Decision**: PlacedRom selects placement under a typed
`PlacementProfile` enum:

```rust
pub enum PlacementProfile {
    StrictOnePerBank,   // bring-up / debug; one expert per bank,
                        // no co-residency, no packing.
    Budgeted,           // default; one expert per bank with
                        // declared slack, common-bank packing
                        // for shared kernels.
    PackedExperts,      // tight; multiple small experts may
                        // co-reside in one bank when legal.
}
```

The profile is selected by `ResolvedCompilePolicy::placement_profile`
and recorded in `policy_resolution.json` (F-B2's territory). F-B15
reads the profile by hash and applies it deterministically.

**Cite**: `planv0.md` lines 1949–1953 (the three named profiles).

**Mechanism**:

1. F-B2 resolves the profile from `CompileRequest.placement_profile`
   (or `ProfileDefault` per the per-profile defaults in
   `planv0.md` line 2557+: `Bringup` ⇒ `StrictOnePerBank`,
   `Default` ⇒ `Budgeted`, `Trace` ⇒ `Budgeted`,
   `Recovery` ⇒ may advance to `PackedExperts`).
2. F-B15's PlacedRom invokes a profile-specific layout strategy:
   `StrictOnePerBank` enforces one expert per bank and rejects
   co-residency at layout time; `Budgeted` packs common-bank
   kernels but keeps experts isolated; `PackedExperts` packs
   experts when a co-residency proof is available.
3. The placement profile is recorded in `placed_rom_plan.json` and
   echoed in `map.json`. The `StageCache` key K14 (PlacedRom)
   includes `placement_profile_hash` (§12.4).
4. F-B16, when it lands, may request a retry under a different
   profile by varying `ResolvedCompilePolicy.placement_profile` and
   re-running F-B15. F-B15 itself **does not** fall back across
   profiles; the fallback happens at the loop-driver layer.

**Rejection class**: `PLACE-PROFILE-INFEASIBLE` (§13.3.7) when the
selected profile cannot place the program; F-B16 turns this into a
`RepairProposal::ProfilePromotion` if its lock-set permits.

### 2.5 The encoder is tiny — all judgment is in PlacedRom

**Decision**: EncodedRom (§11) is a deterministic serializer with
**no choice points and no policy**. Every byte traces to a
`PlacedRom` decision; every symbol traces to a `Section`; every
listing line traces to a `Section`+offset.

**Cite**: `planv0.md` line 1983 ("The encoder should be tiny").
F-A1 RFC §11.1 (cartridge header bytes assembled by the F-A1 ROM
builder; no policy in the encoder).

**Mechanism**:

1. F-A1's `gbf-asm::encoder` is the unique `Instr → bytes` path
   (per F-A1 Rule 2). F-B15's EncodedRom calls it once per
   `Instr` in placement order.
2. F-A1's `gbf-asm::rom` is the cartridge-header assembler. F-B15
   provides the `CartridgeHeader` value (with `BuildIdentityBlock`
   embedded; §11.1.4) and calls `rom::assemble_rom`.
3. F-A1's `gbf-asm::listing::emit_listing` produces the `.lst`. F-B15
   provides the `LayoutPlan` and calls the listing emitter once.
4. F-A1's `gbf-asm::symbols::write_sym` produces the `.sym`. F-B15
   provides the resolved `SymbolTable` and calls the writer once.
5. F-B15 owns no encoding logic. Every byte produced by F-B15 is
   mediated through one of those four F-A1 entry points.

The "tiny encoder" property is preserved by **structural
discipline**: the `EncodedRom` driver in `gbf-codegen::backend` is
≤200 LOC and contains no `match` over `Instr` variants, no
register-encoding tables, and no symbol-resolution logic.

**Rejection class**: `ENC-DRIFT` (§13.4.1) when the encoded bytes
disagree with PlacedRom's expected byte counts; this is a
catastrophic invariant violation, not a routine rejection.

### 2.6 Far-call thunk insertion is part of placement, not a runtime fixup

**Decision**: Cross-bank `CALL <symbol@bank_n>` is rewritten during
placement (§10.2.2) into `CALL <thunk-for-symbol>`, where the
thunk is a Bank0-resident trampoline emitted by F-A4's
`BankingPreLayoutLowering`. The thunk owns both the bank-acquire
sequence and the in-bank jump. There is **no runtime fixup**;
every cross-bank call is bytewise resolved at link/encode time.

**Cite**: F-A1 RFC's Decision 4 ("Far-call thunks live in Bank 0
as per-target-symbol trampolines (`runtime.banking.thunk.<target_symbol>`)").
F-A4 RFC §0 (the four MBC-writing helpers are the only audited
lowering path).

**Mechanism**:

1. AsmIR codegen emits `LegalizationOp::FarCall { bank, target }`
   wherever the IR-level call crosses banks. The pseudo-op is
   placement-dependent (it lowers during legalization, not before
   layout).
2. F-A1's relax/legalization pass discovers the far-call during
   the iterative-monotone fixed point and replaces it with a
   `CALL <thunk-for-symbol>` plus an emitted thunk section.
3. F-A4's `BankingPreLayoutLowering` lowers the thunk body: the
   thunk performs the `lease_rom_switchable(bank)` / inner CALL
   / `release_bank` sequence with the appropriate
   `InterruptPolicy::ShortCriticalSection` discipline.
4. The thunk is named `runtime.banking.thunk.<target_symbol>` and
   appears in `.sym`, `.lst`, and `map.json`. PlacedRom places
   it in Bank 0 as a runtime nucleus extension.
5. F-B15's ReachabilityValidation traces the call edge from the
   originating section through the thunk to the target;
   thunks are reachability-transparent (the thunk's class is the
   meet of caller's and callee's classes).

Far-call thunks are emitted **deterministically per
target-symbol** (one thunk per `(bank, symbol)` pair, not per
call site). The deduplication is owned by F-A4's lowering; F-B15
asserts the property in `placed_rom_plan.json` (§10.7).

**Rejection class**: `PLACE-FAR-CALL-RESIDENCY` (§13.3.8) when a
far-call originates from a section whose residency class forbids
it (e.g. an ISR-reachable section calling into a switchable bank).

### 2.7 Sub-pass ordering is fixed: AsmIR → Reach → Place → Encode

**Decision**: The four sub-passes execute in strict order with no
out-of-order or interleaved variant. AsmIR codegen completes
before ReachabilityValidation begins; ReachabilityValidation
completes before PlacedRom begins; PlacedRom completes before
EncodedRom begins.

**Cite**: `planv0.md` line 1906 ("The backend is a single headline
step that contains **four** internal sub-passes" — listed in this
order). F-A1 RFC §0 ("strictly ordered, deterministic sequence").

**Mechanism**:

1. The driver `run_stage12(SchedulePack, policy, target, nucleus)
   -> Result<Stage12Output, PassDiagnostics>` orchestrates the
   four sub-passes. Each sub-pass is its own function with a
   typed input/output product.
2. Each sub-pass's output type is the input type of the next:
   `AsmIRBundle → ReachabilityReport → PlacedRom → EncodedRom`.
3. The driver does not branch on partial failures: if AsmIR
   codegen fails, ReachabilityValidation never runs; if
   ReachabilityValidation rejects, PlacedRom never runs; etc.
4. Each sub-pass has its own `StageCache` key (§12). The cache
   serves cached outputs for upstream sub-passes when their
   inputs match, regardless of whether downstream sub-passes need
   to re-run.

**Why this ordering is mandatory** (and not, e.g., reachability
after placement):

* Reachability needs the call/branch/thunk graph to be **fully
  legalized**: every branch resolved or rewritten, every far-call
  rewritten to a thunk-call. Far-call legalization happens during
  PlacedRom's relax pass, which depends on PlacedRom-resolved bank
  assignments. So PlacedRom comes after the *first* reachability
  pass conceptually.
* But the ISR residency rule is enforced *during* PlacedRom: a
  section's bank assignment can be rejected because the section
  is `IsrReachable` (per the reachability classification) and the
  bank is switchable.
* Resolution: the reachability pass runs **once** on the
  pre-placement edge graph, classifying every section by the
  classes that depend only on the *call graph topology* (ISR
  reachability, harness reachability, fault reachability) — not on
  bank assignments. The lease-scope class (`BankLeaseProtected`)
  may be pre-seeded from symbolic `BankLease`/`BankRelease`
  boundaries before placement, then carried through PlacedRom and
  checked against final placement. The §9 lattice (§9.1) makes
  this split explicit: topology roots are production roots, while
  lease protection is a seed fact attached to lease-bearing code.
* The four-sub-pass ordering is therefore: AsmIR codegen →
  topological reachability classification → placement (which
  enforces the topological classes against bank assignments and
  computes the bank-derived classes) → encoding.

The strict ordering is enforced structurally by the typed product
chain; out-of-order invocation is a `cargo check` error.

**Rejection class**: N/A — this is a structural invariant of the
driver, not a runtime check.

### 2.8 Determinism: same inputs ⇒ same bytes

**Decision**: F-B15 is a **pure function** of (SchedulePack hash,
ResolvedCompilePolicy hash, TargetProfile hash, runtime nucleus
hash, cartridge header constants, AsmIR-codegen-version,
LayoutAlgorithmVersion, PlacementProfile). Same inputs ⇒
byte-identical `.gb` + byte-identical `.sym` + byte-identical
`.lst` + byte-identical reports.

**Cite**: F-A1 RFC's Decision 5 ("The encoder is the only
function that converts `Instr` to bytes. Bit-stability is asserted
by a property test"). `planv0.md` line 1972 ("bank packing is
deterministic"). F-B11/F-B12 RFC §2 (deterministic byte ordering
under each `PlacementProfile`).

**Mechanism**:

1. Every internal data structure with set/map semantics uses
   `BTreeMap` / `BTreeSet`, not `HashMap` / `HashSet`. Iteration
   order is lex-sorted by key.
2. Every iteration over `Vec<_>` preserves insertion order.
3. Every "choose one of equally good" decision (e.g. which expert
   bank to pick when two banks have equal hotness) is broken by a
   total order on the relevant id type.
4. Symbol naming is canonical: `expert.<expert_id>.section.<index>`,
   `kernel.<kernel_id>.entry`, `runtime.banking.thunk.<symbol>`.
   No timestamps, no host paths, no PID-derived noise.
5. The encoder is bit-stable per F-A1 Decision 5.
6. The output is hash-checked: F-B15 emits `placed_rom_self_hash`
   and `encoded_rom_self_hash` over canonical byte representations.
7. A property test in `gbf-codegen::backend::tests` regenerates a
   fixture build twice on the same checkout and asserts byte
   equality on `.gb`, `.sym`, `.lst`, all reports, and all
   certificates.

**Rejection class**: `ENC-NONDETERMINISM` (§13.4.2) — caught by
the regeneration property test, not by routine compilation.

### 2.9 No new public types from `gbf-asm`, `gbf-hw`, `gbf-abi`,
`gbf-runtime::banking` are introduced here

**Decision**: F-B15 does not extend the F-A1, F-A2, F-A3, or F-A4
public surface. Every type used by F-B15's codegen pipeline is
already in tree (per the cited shipped commits in §1.6, §1.7, §1.8).

**Cite**: F-A1 RFC §0 (the F-A1 surface is the only legal authoring
layer); F-A4 RFC §0 (the four MBC-writing helpers are the only
audited lowering path).

**Mechanism**: This RFC's §3 glossary additions and §13 diagnostic
codes are **F-B15-internal**. They live in `gbf-codegen::backend`
and `gbf-report::backend`; they do not extend any other crate's
public surface.

If F-B15 implementation discovers a missing primitive (e.g. a new
`PreLayoutOp` variant), the addition lands in F-A1 via a separate
bead/PR and a corresponding RFC amendment to F-A1. F-B15 is held
back behind that amendment.

### 2.10 Structural sub-pass output types

**Decision**: Each sub-pass produces a typed product that is the
input of the next. The product chain is:

```rust
// Sub-pass 1: AsmIR codegen
pub struct AsmIRBundle {
    pub nucleus_sections: Vec<gbf_asm::Section>,         // F-A5 input by hash
    pub codegen_sections: Vec<gbf_asm::Section>,         // F-B15 emits
    pub cartridge_header: gbf_asm::rom::CartridgeHeader, // F-A1 ROM builder input
    pub interrupt_safety_table: gbf_runtime::banking::InterruptSafetyTable,
    pub provenance: AsmIRProvenanceMap,                  // §8.5
    pub identity: AsmIRBundleIdentity,                   // hashes + version
    pub report_envelope: ReportEnvelope<AsmIRReportBody>,
}

// Sub-pass 2: ReachabilityValidation
pub struct ReachabilityReport {
    pub class_per_section: BTreeMap<SectionId, ReachabilityClassSet>,
    pub class_per_byte: BTreeMap<(SectionId, u16), ReachabilityClassSet>,
    pub edge_graph_hash: Hash256,                        // §9.2
    pub roots: ReachabilityRoots,                        // §9.2.2
    pub validations: Vec<ReachabilityFinding>,           // §9.3
    pub identity: ReachabilityReportIdentity,
    pub certificate: ReachabilityCertificate,            // §9.5
    pub report_envelope: ReportEnvelope<ReachabilityReportBody>,
}

// Sub-pass 3: PlacedRom
pub struct PlacedRom {
    pub layout: gbf_asm::layout::LayoutPlan,
    pub legalized: Vec<gbf_asm::section::LegalizedSection>,
    pub thunks: Vec<gbf_asm::section::LegalizedSection>, // banking thunks
    pub symbol_table: gbf_asm::symbols::SymbolTable,
    pub placement_profile: PlacementProfile,
    pub bank_assignments: BTreeMap<SectionId, BankIndex>,
    pub map_entries: BTreeMap<u16, MapEntry>,            // for map.json
    pub identity: PlacedRomIdentity,
    pub report_envelope: ReportEnvelope<PlacedRomReportBody>,
}

// Sub-pass 4: EncodedRom
pub struct EncodedRom {
    pub gb_bytes: Vec<u8>,                                // .gb
    pub sym_lines: Vec<String>,                           // .sym (sorted)
    pub lst_text: String,                                 // .lst
    pub identity: EncodedRomIdentity,
    pub report_envelope: ReportEnvelope<EncodedRomReportBody>,
}

// Aggregate
pub struct Stage12Output {
    pub asmir: AsmIRBundle,
    pub reachability: ReachabilityReport,
    pub placed: PlacedRom,
    pub encoded: EncodedRom,
}
```

The product chain is the structural seam that prevents
sub-pass-ordering drift. The driver `run_stage12` is the only
constructor of `Stage12Output`.

### 2.11 No raw byte blobs in compiler-generated sections

**Decision**: F-B15-emitted codegen sections contain only `Instr`,
`DataBlock::Bytes`, `DataBlock::Words`, alignments, labels,
`PreLayoutOp`s, and `LegalizationOp`s — never `Raw(Vec<u8>)`. The
cartridge header bytes (Nintendo logo, header checksum, etc.) are
emitted via F-A1's `gbf-asm::rom` builder using typed `Db`/`Dw`
items.

**Cite**: F-A1 RFC's Rule 10 override ("`SectionItem::Raw` and
`MachineEffect::OpaqueBytes` are removed from `gbf-asm`. Every byte
is authored through `Instr`, `Db`, or `Dw`").

**Mechanism**: F-A1's `Section` storage no longer carries a `Raw`
variant. F-B15's codegen pipeline cannot emit raw bytes because the
type system does not permit it. The cartridge logo and header
checksums travel through `Db`/`Dw` directives whose provenance
points into `gbf-asm::rom` (the cartridge-builder origin).

This is a stricter posture than `planv0.md` line 2537 ("`Raw(Vec<u8>)`
remains legal only as an audited escape hatch for cartridge header
bytes"). F-A1 has tightened the rule and F-B15 inherits the
tightening: there is now nothing left to audit.

**Rejection class**: N/A — structurally impossible; no
diagnostic code needed.

### 2.12 The four reports F-B15 owns

**Decision**: F-B15 emits exactly four reports + one certificate:

| Report                          | Owner    | Schema id              | Notes                               |
|---------------------------------|----------|------------------------|-------------------------------------|
| `placed_rom_plan.json`          | F-B15    | `placed_rom.v1`        | placement decisions + symbol map    |
| `map.json`                      | F-B15    | `map.v1`               | the load-bearing build artifact     |
| `reachability_report.json`      | F-B15    | `reachability.v1`      | per-section class assignment        |
| `certs/reachability.cert.json`  | F-B15    | `reachability_cert.v1` | machine-checkable cert              |

Other reports listed in `planv0.md` §"Reports and artifacts"
(`build_manifest.json`, `provenance.json`, `compiler_feedback.json`,
`hint_consumption.json`, etc.) are owned by **F-F1** (`gbf-report`)
and consume F-B15 outputs by hash. F-B15 does **not** emit them.

The `.gb`, `.sym`, `.lst` artifacts are emitted by F-B15 but are not
"reports" in the canonical-JSON sense — they are the deployable
ROM byte sequence and its debugging companions.

**Cite**: `planv0.md` lines 1985–1987 (build report family);
F-B11/F-B12 §2 (per-stage report ownership).

### 2.13 No profile-time relaxation of validation gates

**Decision**: F-B15 honors `PlacementProfile`, but the profile
**does not relax** any of the seven reachability rules (§9.3) or
the six placement constraints (§10.5). All gates apply uniformly
across all profiles. The profile changes only the **placement
strategy** (one-per-bank vs. budgeted vs. packed); it does not
loosen the safety rules.

**Cite**: `planv0.md` lines 2570–2583 ("`Bringup` is a profile
selection, not a relaxation surface. Profiles do not carry
profile-time relaxations of validation gates"). F-B2/F-B4 §2.13.

**Mechanism**: The validation routines in §9.3 take no profile
parameter. The placement constraints in §10.5 are profile-agnostic
preconditions on a successful placement. The profile selects which
*placement strategy* is invoked; the strategy is constrained to
satisfy the same set of rules regardless.

`Bringup` defaults to `StrictOnePerBank` (per `planv0.md` line
2561's table). That is a *placement* default, not a *relaxation*.

**Rejection class**: N/A — structurally enforced by the type-system
(profile flows into the strategy selection, not into the gate
predicates).

## 3. Glossary additions

This section adds terms to `history/glossary.md` that are introduced or
sharpened by this RFC. None of the terms shadow F-A1, F-A2, F-A3, F-A4,
F-A5, F-B11/F-B12, or F-B13's vocabulary; they are RFC-internal where
necessary and otherwise extend the existing glossary.

### 3.1 ReachabilityClass

A typed enum naming a single reachability path-class for code or
data bytes. The full lattice is in §9.1. The six base classes are
`IsrReachable`, `YieldResumeReachable`, `FaultPathReachable`,
`HarnessEntryReachable`, `BankLeaseProtected`, `NormalOnly`. The
class of a byte is a **set** of classes (`ReachabilityClassSet`)
because the lattice is a join-semilattice — a byte may be
simultaneously `IsrReachable` and `BankLeaseProtected`.

Owner: `gbf-codegen::backend::reachability`. Status: RFC term.

### 3.2 ReachabilityClassSet

A bit-set view over the six base reachability classes. Computed by
the §9 sub-pass; recorded per section and per `(SectionId, byte_offset)`
in `reachability_report.json`.

Owner: `gbf-codegen::backend::reachability`. Status: RFC term.

### 3.3 ReachabilityRoot

A typed entry point to the reachability walk. Five root families:
`InterruptVector(InterruptSource)`, `HarnessEntry(HarnessOp)`,
`ContinuationEntry(RuntimeMode)`, `FaultEntry(FaultDomain)`,
`PanicEntry`. Per §9.2.2.

Owner: `gbf-codegen::backend::reachability`. Status: RFC term.

### 3.4 PlacementProfile

A typed enum selecting the placement strategy used by §10. Three
variants: `StrictOnePerBank`, `Budgeted`, `PackedExperts`. Per §2.4.

Owner: `gbf-codegen::backend::placed`. Status: RFC term.
**Cross-reference**: `planv0.md` lines 1949–1953; `RepairPolicy.
allow_placement_profile_fallback` in `planv0.md` line 2559+.

### 3.5 SchedulePack

The frozen output of F-B13 (Stage 10/10.5). Per `planv0.md` lines
1840–1853:

```rust
pub struct SchedulePack {
    pub modes: BTreeMap<RuntimeMode, GbSchedIR>,
    pub epochs: BTreeMap<RuntimeMode, Vec<ResidencyEpoch>>,
    pub checkpoint_schema_hash: Hash256,
    pub switch_policy: ModeSwitchPolicy,
}
```

F-B15 consumes this by hash. Owner: F-B13. Status: existing.

### 3.6 AsmIRProvenanceMap

A canonical-JSON-serializable map from emitted `(SectionId, ItemIndex)`
pairs to the originating `(SchedSlice, SchedOp, EffectId,
SemanticCheckpointId?, TraceProbeId?, ValueId?, NodeId?)` tuple. Per
§8.5.

Owner: `gbf-codegen::backend::asmir`. Status: RFC term.

### 3.7 PlacedRomIdentity

Hash-bound identity record for a `PlacedRom` product:

```rust
pub struct PlacedRomIdentity {
    pub schedule_pack_hash: Hash256,
    pub resolved_compile_policy_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub asmir_codegen_version: SchemaVersion,
    pub layout_algorithm_version: SchemaVersion,
    pub placement_profile: PlacementProfile,
    pub schedule_cost_report_hash: Hash256,
    pub overlay_plan_hash: Hash256,
    pub arena_plan_hash: Hash256,
    pub rom_window_plan_hash: Hash256,
    pub reachability_report_hash: Hash256,
    pub placed_rom_self_hash: Hash256,
}
```

Owner: `gbf-codegen::backend::placed`. Status: RFC term.

### 3.8 Vector slot

The five DMG interrupt vectors at `$0040`, `$0048`, `$0050`,
`$0058`, `$0060`. Per F-A2's `gbf_hw::interrupts::INT_VECTOR_*`.
F-B15 places vectors as first-class layout entities (§2.2).

Owner: F-A2 (constants); F-A5 (stub bodies); F-B15 (placement).
Status: existing in F-A2; reachability-root status added here.

### 3.9 Far-call thunk

A Bank-0-resident trampoline emitted by F-A4's
`BankingPreLayoutLowering` to legalize a cross-bank call. One thunk
per `(target_bank, target_symbol)` pair. Symbol name:
`runtime.banking.thunk.<target_symbol>`. Per §2.6, §10.2.2.

Owner: F-A4 (lowering); F-B15 (placement). Status: existing in F-A4
RFC; re-stated here.

### 3.10 Continuation entry

The per-`RuntimeMode` AsmIR section that the cooperative scheduler
returns to after a yield. Each `RuntimeMode` keyed under
`SchedulePack.modes` has its own continuation entry. Per §8.3.

Owner: `gbf-codegen::backend::asmir`. Status: RFC term.

### 3.11 Codegen front-end

The IR-to-AsmIR transformer: the F-B15 sub-pass (§8) that produces
a `Vec<Section>` from a `SchedulePack`. Distinct from F-A1's typed
authoring layer, which is the codegen *back-end* (the Builder API
and Encoder). The codegen front-end is owned by F-B15; the codegen
back-end is owned by F-A1.

Owner: `gbf-codegen::backend::asmir`. Status: RFC term.

### 3.12 Edge graph

The directed graph used as input to ReachabilityValidation. Nodes
are `(SectionId, item_index)` pairs (or, for fall-through edges,
just `SectionId`). Edges are typed: `Call`, `JumpRelative`,
`JumpAbsolute`, `FarCallViaThunk`, `FallThrough`, `InterruptReturn`,
`PanicJump`, `RstVector`. Constructed after far-call legalization
so every edge is byte-resolvable.

Owner: `gbf-codegen::backend::reachability`. Status: RFC term.

### 3.13 ResidencyEpoch (consumed)

Per `planv0.md` lines 1832–1838 and F-B13's RFC:

```rust
pub struct ResidencyEpoch {
    pub id: EpochId,
    pub rom_window: RomWindowBinding,
    pub overlay: Option<OverlayId>,
    pub residency: Residency,
    pub slices: Vec<SliceId>,
}
```

F-B15 consumes one `ResidencyEpoch` per slice-group per
`RuntimeMode`. Each epoch has a single concrete bank assignment,
which constrains placement. Owner: F-B13. Status: existing.

### 3.14 BuildIdentityBlock (consumed + emitted)

Per F-A3 §3.1:

```rust
#[repr(C)]
pub struct BuildIdentityBlock {
    pub magic: [u8; 4],         // b"GBLM"
    pub abi: AbiVersion,
    pub _pad0: u8,
    pub build_hash: [u8; 32],
    pub artifact_core_hash: [u8; 32],
    pub runtime_nucleus_hash: [u8; 32],
    pub compile_request_hash: [u8; 32],
    pub build_unix_timestamp_ms: u64,
    pub continuation_tail_bytes: u32,
}
```

F-B15 fills every field and emits the block at the cartridge-header
offset declared by F-A5 / F-A1. Owner: F-A3 (type); F-B15 (emission).
Status: existing.

### 3.15 SymbolName (consumed)

F-A1's `gbf_asm::symbols::SymbolName`: validated dotted canonical
names with constructor families `kernel`, `expert`, `runtime`,
`section`. F-B15 mints names per the canonical scheme in §10.2.4
and §11.2; it does not extend the constructor families.

Owner: F-A1. Status: existing.

### 3.16 InterruptSafetyTable (consumed)

F-A4's `gbf_runtime::banking::InterruptSafetyTable`: a typed map
from `SectionId` to `InterruptSafetyKind`
(`InterruptDisabled` / `InterruptEnabledBank0Only` / `InterruptHandler`).
F-A5 is the first emitter; F-B15's codegen extends the table with
declarations for codegen-emitted sections and presents the union
to ReachabilityValidation as one of the declared-annotation inputs
(§9.6).

Owner: F-A4. Status: existing.

### 3.17 PlacedRom certificate

The machine-checkable certificate emitted alongside
`reachability_report.json`. Schema id `reachability_cert.v1`. Per
§9.5.

Owner: `gbf-codegen::backend::reachability`. Status: RFC term.

### 3.18 MapEntry

A canonical-JSON record describing one named region of the encoded
ROM (a section, an arena, a thunk, the cartridge header, a vector
slot, a persistent SRAM page). Per §10.8.

```rust
pub struct MapEntry {
    pub kind: MapEntryKind,
    pub address_space: AddressSpace,
    pub start: u32,                   // absolute or bank-local
    pub size_bytes: u32,
    pub bank: Option<BankIndex>,
    pub residency: Residency,
    pub privilege: PrivilegeClass,
    pub reachability_classes: ReachabilityClassSet,
    pub provenance_summary: MapEntryProvenance,
    pub symbol: Option<SymbolName>,
    pub cycles_estimate: Option<CycleBudget>,  // from F-B14
    pub bank_switches_estimate: Option<u32>,   // from F-B14
}
```

Owner: `gbf-codegen::backend::placed`. Status: RFC term.

### 3.19 LayoutAlgorithmVersion

A `SchemaVersion` constant pinning the F-A1 layout/relax algorithm
release. Bumped when the layout/relax pass changes its
fixed-point ordering, thunk-deduplication discipline, or
section-ordering rule. Included in PlacedRom's StageCache key
(§12.4) so a layout-algorithm change invalidates cached PlacedRoms.

Owner: F-A1. Status: existing as `LAYOUT_ALGORITHM_VERSION`
constant in `gbf-asm`; consumed here.

### 3.20 AsmIRCodegenVersion

A `SchemaVersion` constant pinning F-B15's codegen front-end
release. Bumped when the slice-to-AsmIR lowering rules change.
Included in AsmIR's StageCache key (§12.2).

Owner: F-B15 (`gbf-codegen::backend::asmir`). Status: RFC term.

## 4. Core notation

### 4.1 Hash256 / DomainHash / SelfHash / ZERO_HASH

Inherited from F-B2/F-B4 §1, unchanged. In particular:

```text
DomainHash(crate, type, schema_id, schema_version, canonical_json_bytes)
  = SHA256(format!("gbf:{}:{}:{}:{}\0", crate, type, schema_id, schema_version)
           ++ canonical_json_bytes)

SelfHash(report) = DomainHash(.., canonical_json_with_self_hash_field_set_to(ZERO_HASH))

ZERO_HASH = Hash256("0000000000000000000000000000000000000000000000000000000000000000")
```

Lowercase hex; the `sha256:` prefix is part of the JSON schema, not
of the digest input bytes.

### 4.2 CanonicalJson(x)

Inherited from F-B2/F-B4 §1, unchanged. UTF-8 byte stream;
lex-sorted object keys; integers only (no floats, no NaN/Inf,
no scientific); explicit enum tags; no unknown fields; arrays
preserve declared order; strings are JSON-escaped per RFC 8259.

### 4.3 Stage12 driver shape

```text
run_stage12(SchedulePack, ResolvedCompilePolicy, TargetProfile,
            RuntimeNucleusBundle, F-B14 ScheduleCostReport, env)
  -> Result<Stage12Output, PassDiagnostics>

  where:
    RuntimeNucleusBundle ::= {
      sections: Vec<gbf_asm::Section>,         // F-A5 nucleus
      cartridge_header: gbf_asm::rom::CartridgeHeader,
      runtime_nucleus_hash: Hash256,
      build_identity_args: BuildIdentityArgs,
    }

  driver pseudocode:
    1. let asmir = run_asmir_codegen(sched, policy, target, nucleus)?;
    2. let reach = run_reachability(asmir)?;
    3. let placed = run_placed_rom(asmir, reach, policy.placement_profile, costs)?;
    4. let encoded = run_encoded_rom(placed)?;
    5. emit reports + certificates.
    6. write StageCache entries (K12, K13, K14, K15).
    7. return Stage12Output { asmir, reach, placed, encoded }.
```

Each sub-pass is a pure function from typed inputs to a typed
output product plus a `ReportEnvelope`. The driver wraps with IO
(StageCache + report emission). Determinism is required, not
aspirational.

### 4.4 Pure-core / driver split

Each sub-pass has two layers, mirroring F-B3/F-B5 §2.1:

```text
asmir_codegen_core(AsmIRInputs)
  -> Result<(AsmIRBundle, ReportEnvelope<AsmIRReportBody>), PassDiagnostics>

run_asmir_codegen(AsmIRInputs, env)
  = asmir_codegen_core(...) then
    (on success or failure):
      emit asmir_summary.json (debug-only, gated behind cfg)
      may write StageCache success entry
      may write StageCache failure memo
```

Cores never mutate inputs. Drivers are the only IO surface. The
chunk-level pass shape per sub-pass is:

```text
PassInputs (pinned, hash-bound)
  -> Pure Core
       (typed transformations / typed proofs / typed placements)
  -> Result<PassOutputs, PassDiagnostics>
       PassOutputs := { typed product, ReportEnvelope<ReportV1> }
       PassDiagnostics := list of typed ValidationDiagnostic
  -> Driver (IO)
       emits canonical JSON
       writes StageCache success / failure memo
```

### 4.5 Report outcomes

Inherited from F-B2/F-B4 §2.1. `ReportOutcome::Passed` /
`ReportOutcome::Failed`. F-B15 reports reject `Soft` diagnostics
(`R-HardOnly-ThisChunk`); every expected reject case is a hard stop.

### 4.6 Diagnostic shape

Inherited from F-B2/F-B4 §2.2:

```rust
pub struct ValidationDiagnostic {
    pub severity: DiagnosticSeverity,   // Hard | Soft
    pub origin: ValidationOrigin,
    pub code: ValidationCode,
    pub detail: ValidationDetail,
    pub provenance: Vec<EvidenceRef>,
}
```

F-B15 introduces four new `ValidationOrigin` variants:
`AsmIRCodegen`, `ReachabilityValidation`, `PlacedRomLayout`,
`EncodedRomEmission` (§13). Diagnostic codes are detailed in §13.

## 5. Authority rules

This RFC introduces the following authority claims; later RFCs
that change any claimed surface must amend this RFC explicitly.

### 5.1 Owned by F-B15 (this RFC)

* The codegen front-end pipeline that produces `Vec<gbf_asm::Section>`
  from a `SchedulePack` (§8).
* The whole-program reachability lattice and the seven validation
  rules (§9).
* The `PlacementProfile` enum's *consumption* — i.e. how F-B15
  applies the profile to placement strategy (§10). The enum
  *definition* itself sits in `gbf-policy` so that F-B2's
  `ResolvedCompilePolicy` can carry it without depending on
  `gbf-codegen`. F-B15 does not extend the enum.
* The reachability certificate shape (`reachability_cert.v1`,
  §9.5).
* The four reports' schemas: `placed_rom.v1`, `map.v1`,
  `reachability.v1`, `reachability_cert.v1` (§10.7, §10.8, §9.5).
* The four StageCache key shapes K12, K13, K14, K15 (§12).
* The four diagnostic-code families ASM-*, REACH-*, PLACE-*,
  ENC-* (§13).
* The Stage 12 driver shape (§4.3).
* The strict sub-pass ordering invariant (§2.7).
* The determinism contract (§2.8).
* The end-to-end theorem (§18).

### 5.2 Inherited unchanged

Each item names the precise prior RFC section so a future amendment
to that prior RFC cannot silently weaken what this RFC depends on.

* `ReportEnvelope<R>` shape — F-B2/F-B4 §4.
* `Hash256`, `DomainHash`, `SelfHash`, `ZERO_HASH` — F-B2/F-B4 §1.
* `CanonicalJson(x)` rule — F-B2/F-B4 §1.
* `null` policy — F-B2/F-B4 §1.
* Envelope laws (`R-Hash`, `R-Outcome-Pass`, `R-Outcome-Fail`,
  `R-FlatEnvelope`, `R-UnknownReject`) — F-B2/F-B4 §4.
* `ValidationDiagnostic` shape — F-B2/F-B4 §5.
* `R-HardOnly-ThisChunk` — F-B2/F-B4 §4.
* Diagnostic laws (`D-CodeClosed`, `D-NoStringOnly`, `D-Renderable`,
  `D-Provenance`) — F-B2/F-B4 §5.
* StageCache key construction rule — F-B2/F-B4 §11.
* AsmIR types (`Instr`, `Section`, `SectionRole`, `MachineEffect`,
  `PrivilegeClass`, `Builder`, `PreLayoutOp`, `LegalizationOp`,
  `OrderedItem<T>`, `LoweredSection`, `LegalizedSection`) — F-A1
  §2, §3, §4.
* `gbf-asm::encoder` — F-A1 §6.
* `gbf-asm::layout`, `gbf-asm::relax` — F-A1 §4, §5.
* `gbf-asm::rom` — F-A1 §11.
* `gbf-asm::symbols` — F-A1 §10.
* `gbf-asm::listing` — F-A1 §9.
* `gbf-hw::cartridge_header` — F-A2 §3A.
* `gbf-hw::memory` (region map, predicates) — F-A2 §3.
* `gbf-hw::mbc5` (register addresses, RAM-enable token) — F-A2 §4.
* `gbf-hw::interrupts` (`INT_VECTOR_*`, `IE_REGISTER`, `IF_REGISTER`)
  — F-A2 §6.
* `gbf-abi::version::BuildIdentityBlock` (layout, magic, hash
  fields, timestamp) — F-A3 §3.1.
* `gbf-abi::version::AbiVersion`, `CURRENT_ABI` — F-A3 §3.1.
* `gbf-abi::version::CompatibilityEnvelope` — F-A3 §3.1.
* `gbf-abi::checkpoint::SemanticCheckpointId`,
  `CompactCheckpointId`, `SemanticCheckpointSchema` — F-A3 §3.6.
* `gbf-abi::interrupt::InterruptPolicy`, `ResourceLease`,
  `ResourceLeaseKind` — F-A3 §3.5.
* `gbf-abi::fault::FaultCode`, `FaultDomain`,
  `classify_fault`, `FaultSnapshot` — F-A3 §3.4.
* `gbf-abi::harness::HarnessCommandBlock`,
  `HarnessResultBlock`, `HarnessOp` — F-A3 §3.3.
* `gbf-abi::trace::TraceEvent` — F-A3 §3.7.
* `gbf-runtime::banking` user-facing API
  (`lease_rom_switchable`, `lease_sram`, `release_bank`,
  `lower_banking_shadow_zero_init`, HRAM constants,
  `InterruptSafetyTable`, `mbc_write_provenance_audit`,
  `BankingPreLayoutLowering`) — F-A4 §3, §4, §5.
* F-A5 nucleus section family (boot, interrupts, scheduler,
  joypad, text, keyboard, video_commit, panic) — F-A5 §3A–§3G.
* `RuntimeShellModule` enum — F-A5 §1.1.x.
* `runtime_nucleus_hash` normalization — F-A5 §3H.3.
* `SchedulePack`, `GbSchedIR`, `SchedSlice`, `SchedOp`,
  `ResidencyEpoch`, `ModeSwitchPolicy`, `ResourceVector` —
  F-B13 (RFC forthcoming; consumed by hash here).
* `certs/resource_state.cert.json` — F-B13.
* `ScheduleCostReport`, `EstimatedCostDelta` — F-B14.
* `OverlayPlan`, `ArenaPlan`, persistent-page geometry —
  F-B11/F-B12.
* `RomWindowPlan`, `KernelResidency` — F-B10.
* `ResolvedCompilePolicy`, `PolicyProvenance`, `RuntimeChromeBudget`
  — F-B2/F-B4.
* `QuantGraph`, `GbInferIR` — F-B3/F-B5 (consumed transitively
  through `SchedulePack`'s provenance fields).

### 5.3 Out-of-scope (forwarded to other RFCs)

* `RepairProposal`, `ConstraintDelta`, refinement-loop driver —
  F-B16.
* StageCache uniformization across all stages — F-B17.
* Build-report aggregation (`build_manifest.json`,
  `provenance.json`, `compiler_feedback.json`) — F-F1.
* Independent reachability cross-validator — F-F2 / `gbf-verify`.
* `ScheduleOracle` correspondence — F-C3.
* `ConformanceEnvelope` — F-C4.
* `gbf-debug` session-file consumption — F-A8.
* `gbf-emu` deterministic execution — F-A7.

### 5.4 Pluggable surfaces

The following surfaces are pluggable in the sense that their
*identity is fixed by this RFC* but their *implementation may
change* without an amendment, provided the implementation passes
the closure conditions in §0a:

* The slice-to-AsmIR codegen rules (§8) — implementation may
  change as long as the per-slice section shape (§8.3) and the
  provenance map shape (§8.5) are preserved.
* The placement strategy per profile (§10) — implementation may
  change as long as the six global constraints (§10.5) hold and
  the deterministic byte-ordering invariant (§2.8) is preserved.
* The reachability walker's algorithmic shape (§9.4) — must
  produce the same lattice classification as a reference
  forward-flow analysis on the edge graph; the algorithm itself
  is implementation-defined.

The following surfaces are **not** pluggable; they are pinned
schema:

* `placed_rom.v1`, `map.v1`, `reachability.v1`,
  `reachability_cert.v1` JSON shapes.
* The four StageCache key constructions (§12).
* The four diagnostic-code families and their renderable detail
  variants.
* The end-to-end theorem (§18).

## 6. Pipeline state machine

This section documents how Stage 12 plugs into Stages 11
(`ScheduleCostAnalysis`) and the build-report aggregation step
(downstream).

### 6.1 Inputs (frozen)

```text
SchedulePack
  source: F-B13 GbSchedIR + ResourceStateValidation (Stage 10/10.5)
  identity: schedule_pack_hash
  fields: modes, epochs, checkpoint_schema_hash, switch_policy

certs/resource_state.cert.json
  source: F-B13
  identity: cert hash
  fields: lease balance proof, ISR-state proof, overlay-shadow proof

ResolvedCompilePolicy
  source: F-B2 (Stage 0.5)
  identity: resolved_compile_policy_hash
  fields: CompileObjective, PlacementProfile, CompileKnobs (with bounds
          and lock-set), TargetProfileId, calibration set refs,
          determinism class

TargetProfile
  source: F-A2 (gbf-hw::target)
  identity: target_profile_hash
  fields: ConsoleModel, CartridgeProfile, TimingProfile, CapabilitySet

RuntimeNucleusBundle (composed from F-A5 + F-A1 + F-A3)
  source: F-A5 emits sections; F-A1 ROM builder assembles header;
          F-A3 supplies BuildIdentityArgs
  identity: runtime_nucleus_hash + cartridge_header_hash + abi_version
  fields: nucleus Vec<Section>, CartridgeHeader value,
          BuildIdentityArgs (artifact_core_hash, runtime_nucleus_hash,
          compile_request_hash; build_hash filled in by F-B15)

ScheduleCostReport
  source: F-B14 (Stage 11)
  identity: schedule_cost_report_hash
  fields: per-mode EstimatedCostDelta envelopes

OverlayPlan / ArenaPlan / RomWindowPlan (consumed transitively via
                                          SchedulePack)
  source: F-B11 / F-B12 / F-B10
  identity: overlay_plan_hash, arena_plan_hash, rom_window_plan_hash
```

All inputs are content-addressed. F-B15 reads them by hash; if any
input hash does not match the SchedulePack's recorded
`upstream_hashes` field, the build fails fast with
`STAGE12-INPUT-HASH-MISMATCH` (§13.5.1).

### 6.2 Outputs

```text
On success:
  Stage12Output {
    asmir: AsmIRBundle,
    reachability: ReachabilityReport,
    placed: PlacedRom,
    encoded: EncodedRom,
  }

  Reports written:
    placed_rom_plan.json       (placed_rom.v1)
    map.json                   (map.v1)
    reachability_report.json   (reachability.v1)
    certs/reachability.cert.json (reachability_cert.v1)

  Artifacts written:
    game.gb       (the encoded ROM)
    game.sym      (RGBDS-compatible symbol map; sorted)
    game.lst      (interleaved listing)

  StageCache entries written:
    K12 (AsmIR codegen) -> AsmIRBundle by canonical hash
    K13 (Reachability)  -> ReachabilityReport by canonical hash
    K14 (PlacedRom)     -> PlacedRom by canonical hash
    K15 (EncodedRom)    -> EncodedRom by canonical hash

On failure:
  PassDiagnostics with at least one Hard diagnostic.
  Reports may still be written for the sub-passes that succeeded;
  the failed sub-pass writes a Failed-outcome report when enough
  identity is available.
```

### 6.3 Sub-pass state transitions

```text
[Stage 11 done]
     |
     v
[Stage 12 entry]                                  Driver receives all
     |                                            inputs (§6.1).
     v
[K12 cache lookup]
     |
     +- hit -> [AsmIR ready]
     +- miss -> [run_asmir_codegen]
                     |
                     +- ok   -> [AsmIR ready]
                     +- err  -> [Stage 12 fail / write asmir summary]
     v
[K13 cache lookup]                                K13 includes
     |                                            asmir_bundle_hash.
     +- hit -> [Reachability ready]
     +- miss -> [run_reachability]
                     |
                     +- ok   -> [Reachability ready / emit
                                   reachability_report.json +
                                   reachability.cert.json]
                     +- err  -> [Stage 12 fail / emit failed
                                   reachability_report.json]
     v
[K14 cache lookup]                                K14 includes
     |                                            asmir_bundle_hash +
     |                                            reachability_report_hash +
     |                                            placement_profile.
     +- hit -> [PlacedRom ready]
     +- miss -> [run_placed_rom]
                     |
                     +- ok   -> [PlacedRom ready / emit
                                   placed_rom_plan.json + map.json]
                     +- err  -> [Stage 12 fail / emit failed
                                   placed_rom_plan.json]
     v
[K15 cache lookup]                                K15 includes
     |                                            placed_rom_hash +
     |                                            encoder_version +
     |                                            cartridge_header_hash.
     +- hit -> [EncodedRom ready]
     +- miss -> [run_encoded_rom]
                     |
                     +- ok   -> [EncodedRom ready / write .gb,
                                   .sym, .lst]
                     +- err  -> [Stage 12 fail / encoder is tiny,
                                   only ENC-DRIFT or ENC-NONDETERMINISM
                                   reach this branch]
     v
[Stage 12 done]                                   Stage12Output
                                                  returned to caller.
```

### 6.4 Failure modes

A Stage 12 failure has four classes corresponding to the four
sub-passes:

* **AsmIR codegen failure** — typed `ASM-*` diagnostics
  (§13.1). Common causes: lowering rule for a `SchedOp` variant
  is missing, kernel binding fails because `KernelRegistry` does
  not contain a required kernel, runtime nucleus and codegen
  sections collide on a SymbolName.
* **Reachability failure** — typed `REACH-*` diagnostics (§13.2).
  Common causes: ISR-reachable code/data depends on switchable
  bank, declared annotation disagrees with computation, fault
  path depends on non-resident data.
* **Placement failure** — typed `PLACE-*` diagnostics (§13.3).
  Common causes: section too large for any bank slot, expert
  count exceeds available banks under the selected profile,
  arena overflow, branch out of range that cannot be relaxed,
  far-call from forbidden residency.
* **Encoding failure** — typed `ENC-*` diagnostics (§13.4).
  Should never happen in practice; reaching this branch means a
  catastrophic invariant violation upstream.

In every case, F-B15 emits the report for the sub-pass that
failed (if enough identity is available) and propagates the
diagnostics. F-B16, when it lands, may turn certain diagnostic
classes into `RepairProposal` invocations; F-B15 itself does not
loop.

### 6.5 Refinement-loop integration (forward-compat)

F-B15 does **not** drive the loop. F-B16's loop driver, when it
lands, calls F-B15 with varied inputs:

* `PlacementProfile` may be promoted (`Budgeted` → `PackedExperts`)
  if the previous attempt produced `PLACE-PROFILE-INFEASIBLE`.
* `RomWindowPlan`'s `KernelResidency::WramOverlay` may be granted
  if the previous attempt produced
  `PLACE-EXPERT-COMMON-BANK-PRESSURE` (an arena-bank constraint).
* `ObservationPlan`'s probe density may be reduced if the
  previous attempt produced
  `REACH-FAULT-PATH-NONRESIDENT-DATA` because tracing tags
  inflated the fault path.

Each varied input changes upstream-stage hashes, which change
F-B15's input hashes, which invalidates K12/K13/K14/K15. F-B15 is
re-run end-to-end; the cache serves no shortcut on a refinement
retry. This is intentional: refinement decisions cross sub-pass
boundaries, and partial cache reuse would be a correctness hazard.

### 6.6 What this stage is NOT (re-stated for the state machine)

* It is not a transform that mutates `SchedulePack`. Stage 12
  consumes `SchedulePack` by hash and never modifies it.
* It is not a refinement loop. Stage 12 runs once per input
  tuple. F-B16 owns the loop.
* It is not a publisher. Reports/certificates/artifacts are
  written to local paths under `artifacts/builds/<build_hash>/`;
  publishing to remote stores is owned by deployment tooling.
* It is not an oracle. F-C3 (ScheduleOracle) compares the
  encoded ROM against a `GbSchedIR` interpretation; F-B15
  produces the comparable artifact but does not implement the
  comparison.

## 7. Report envelope

This section pins the report envelope shape inherited from
F-B2/F-B4 §4 and applied to F-B15's four reports.

### 7.1 ReportEnvelope shape (inherited)

```rust
pub struct ReportEnvelope<R> {
    pub schema_id: ReportSchemaId,
    pub schema_version: SchemaVersion,
    pub outcome: ReportOutcome,
    pub report_self_hash: Hash256,    // SelfHash convention
    pub identity: ReportIdentity,     // canonical input-hash bundle
    pub diagnostics: Vec<ValidationDiagnostic>,
    pub body: R,                      // schema-specific fields
}
```

### 7.2 The four reports' schema ids

| Report                         | schema_id             | schema_version |
|--------------------------------|----------------------|----------------|
| `placed_rom_plan.json`         | `placed_rom.v1`      | `1.0.0`        |
| `map.json`                     | `map.v1`             | `1.0.0`        |
| `reachability_report.json`     | `reachability.v1`    | `1.0.0`        |
| `certs/reachability.cert.json` | `reachability_cert.v1` | `1.0.0`      |

A breaking change to any schema bumps the major version. Minor
version bumps are reserved for additive fields with declared
defaults; F-B15 does not preemptively reserve any fields for
minor bumps.

### 7.3 Identity field shape per report

Each report's `identity` carries the canonical hashes of every
load-bearing input visible at that report's emission point:

```text
PlacedRomPlanIdentity = PlacedRomIdentity (§3.7)

MapIdentity = same as PlacedRomIdentity + a top-level
              encoded_rom_self_hash field (since map.json describes
              the encoded ROM's address space by absolute address;
              the encoded artifact must exist for map.json to be
              meaningful)

ReachabilityReportIdentity {
    schedule_pack_hash, resolved_compile_policy_hash,
    target_profile_hash, runtime_nucleus_hash,
    asmir_bundle_hash, asmir_codegen_version,
    reachability_walker_version,
}

ReachabilityCertIdentity {
    reachability_report_hash, walker_version,
    compute_summary_hash,
}
```

### 7.4 Self-hash policy

Per F-B2/F-B4 §2.4, every report computes `report_self_hash` over
its canonical-JSON form with the field temporarily set to
`ZERO_HASH`. The four reports follow this rule unchanged.

The certificate `certs/reachability.cert.json` additionally
includes a `validator_witness_hash` field: a hash of a
machine-checkable proof script (or a witness data structure) that
an independent walker can use to verify the cert. F-B15 emits the
witness; the walker (in `gbf-verify`, owned by F-F2) consumes it.

### 7.5 Outcome policy

Per F-B2/F-B4 §2.1:

* `ReportOutcome::Passed` when no `Hard` diagnostics.
* `ReportOutcome::Failed` when at least one `Hard` diagnostic.
* `Soft` diagnostics rejected (`R-HardOnly-ThisChunk`).

For the four F-B15 reports:

* `placed_rom_plan.json` outcome reflects PlacedRom sub-pass
  success/failure.
* `map.json` outcome reflects PlacedRom + EncodedRom combined
  (since `map.json` is meaningless without bytes). If PlacedRom
  fails, `map.json` is not emitted (no entry to populate). If
  EncodedRom fails (catastrophic), `map.json` is emitted with
  outcome `Failed` and the diagnostic enumerated.
* `reachability_report.json` outcome reflects
  ReachabilityValidation sub-pass success/failure.
* `certs/reachability.cert.json` is emitted only when
  `reachability_report.json` is `Passed`. A failed reachability
  pass produces no certificate.

### 7.6 Provenance fields

Every report includes a `provenance` field whose shape depends on
the report:

* `placed_rom_plan.json` carries placement-decision provenance:
  for every section, the
  `(SchedSlice, SchedOp, EffectId, SemanticCheckpointId?)` tuple
  the section originated from (or `RuntimeNucleus(F-A5 module)`
  for nucleus sections).
* `map.json` carries the same provenance plus arena-source
  references (which `ArenaSlot` from `ArenaPlan` claimed each
  WRAM/SRAM region).
* `reachability_report.json` carries
  `(SectionId, item_index) -> ReachabilityClassSet` plus a
  `roots` field naming every reachability root.
* `certs/reachability.cert.json` carries a per-class summary
  plus a `findings` field listing every validation that ran and
  passed.

## 8. Sub-pass 1: AsmIR codegen

### 8.1 Codegen contract

The AsmIR codegen is a pure function:

```rust
pub fn asmir_codegen_core(
    sched: &SchedulePack,
    policy: &ResolvedCompilePolicy,
    target: &TargetProfile,
    nucleus: &RuntimeNucleusBundle,
    cost: &ScheduleCostReport,
) -> Result<(AsmIRBundle, ReportEnvelope<AsmIRReportBody>), PassDiagnostics>;
```

The contract has six parts.

#### 8.1.1 One slice = one section group

Per `planv0.md` line 1908+, the AsmIR codegen lowers each
`SchedSlice` into a section group. A "section group" is a small
fixed shape:

```text
slice.<mode>.<slice_id>.entry        // SectionRole::CommonKernel or
                                     // ExpertPayload(eid), depending on
                                     // SchedOp residency. Holds the
                                     // slice's actual ops.

slice.<mode>.<slice_id>.continuation // SectionRole::RuntimeBank0. The
                                     // continuation header that the
                                     // scheduler returns to after a
                                     // yield. Receives the
                                     // ContinuationFrame from
                                     // gbf_abi::continuation.
```

For a `SchedSlice` that does not yield (`yield_kind == Finished` or
`yield_kind == Fault` without intermediate yields), the
continuation section is omitted; the entry section is the entire
group.

#### 8.1.2 Per-mode continuation entry

Each `RuntimeMode` keyed under `SchedulePack.modes` gets a
**mode-level continuation entry** in addition to the per-slice
continuations:

```text
mode.<mode>.continuation_entry       // SectionRole::RuntimeBank0. The
                                     // single dispatch point the
                                     // scheduler enters when starting
                                     // or resuming a token under that
                                     // mode. Looks up the
                                     // ContinuationFrame's
                                     // current_slice_id and dispatches
                                     // to slice.<mode>.<slice_id>.entry
                                     // or slice.<mode>.<slice_id>.continuation.
```

The mode-level continuation entry is the only Bank0-resident
inference dispatch surface. It has `PrivilegeClass::Normal` and
`SectionRole::RuntimeBank0`.

#### 8.1.3 Per-`ResidencyEpoch` epoch trampoline

For every `ResidencyEpoch` listed in `SchedulePack.epochs[mode]`,
codegen emits an **epoch trampoline** that enters the epoch's slice
list under the declared `RomWindowBinding` and (optional)
`OverlayId`:

```text
epoch.<mode>.<epoch_id>.trampoline    // SectionRole::RuntimeBank0. Acquires
                                      // the epoch's BankGuard via F-A4's
                                      // lease_rom_switchable, optionally
                                      // installs the overlay via F-B11's
                                      // OverlayInstaller, and falls
                                      // through into the first slice
                                      // entry of the epoch.
```

The trampoline is the only place a `BankLease` is acquired by
codegen-emitted code. Per-slice entries do **not** acquire leases;
the epoch trampoline does it once and the trampoline's lifetime
covers the epoch's slice list. This matches `planv0.md`'s "epoch
boundary" semantics (line 1832+).

#### 8.1.4 Per-expert entry stub

For every `ExpertId` referenced by any `SchedOp` in any slice,
codegen emits an **expert entry stub** in that expert's bank:

```text
expert.<expert_id>.entry              // SectionRole::ExpertPayload(eid).
                                      // PrivilegeClass::Normal. Receives
                                      // input pointer in BC, output
                                      // pointer in DE, returns to caller.
                                      // Body is the expert's compiled
                                      // forward pass.
```

The stub itself is small (≤ 64 bytes typically). The expert's
tensor payload travels alongside it as `Db`/`Dw` data directives,
also under `SectionRole::ExpertPayload(eid)`.

#### 8.1.5 Cartridge header + BuildIdentityBlock

The cartridge header section is owned by F-A1's `gbf-asm::rom`
builder; F-B15 supplies the `CartridgeHeader` value (with
title, MBC type, ROM/RAM size codes, destination code, mask ROM
version) and the `BuildIdentityBlock` to embed at the F-A3-defined
offset.

The `BuildIdentityBlock` lives at a known address per F-A5's boot
section layout. F-B15 fills:

* `magic = b"GBLM"` (per F-A3 §3.1.3).
* `abi = CURRENT_ABI` (per F-A3).
* `build_hash` — computed at the end of EncodedRom over the
  encoded `.gb` with the four lineage hashes zeroed (the
  "self-hash with self-hash zeroed" idiom). This is the
  `build_self_hash`.
* `artifact_core_hash` — read from `ResolvedCompilePolicy.
  artifact_core_hash`.
* `runtime_nucleus_hash` — read from `RuntimeNucleusBundle.
  runtime_nucleus_hash`.
* `compile_request_hash` — read from `ResolvedCompilePolicy.
  compile_request_hash`.
* `build_unix_timestamp_ms` — set to a deterministic value: when
  `policy.determinism_class != BitExact`, this is the actual
  system UTC time at codegen start; when
  `policy.determinism_class == BitExact`, this is the
  policy-declared `build_unix_timestamp_ms` (which the
  CompileRequest may pin for reproducibility). The default
  pinned value is `0`.
* `continuation_tail_bytes` — read from
  `gbf_abi::version::BuildIdentityArgs.continuation_tail_bytes`,
  which the per-build sequence-state owner (F-D1 / F-B5) declares
  upstream.

The `build_hash` is filled in **last** because it depends on every
other byte in the ROM. The encoder writes the field as
`ZERO_HASH`, hashes the ROM, then patches the field in place. This
is the same self-hash idiom F-B2/F-B4 uses for canonical-JSON
reports, applied to a binary ROM.

#### 8.1.6 KernelRegistry binding

Codegen consumes a `KernelRegistry` (owned by Epic H, F-H1) that
maps `(KernelSpecId, ResidencyClass) -> Section`. For every
`SchedOp::CallKernel { kernel_id, .. }` in a slice, codegen looks
up the kernel's compiled section and emits a typed call (a
`CALL <kernel_symbol>` for in-bank, or a `LegalizationOp::FarCall`
for cross-bank).

`KernelRegistry` is **input** to F-B15; its construction and
content is owned upstream. Missing kernels produce
`ASM-KERNEL-NOT-FOUND` (§13.1.5).

### 8.2 Pseudo-op surface used

F-B15-emitted codegen sections use the following pseudo-ops, all
owned by F-A1. Each one's typed contract reference points to the
F-A1 RFC.

| Pseudo-op                      | Owner | Use in F-B15                                              |
|--------------------------------|-------|-----------------------------------------------------------|
| `PreLayoutOp::BankLease`       | F-A1  | Epoch trampoline acquires its bank.                       |
| `PreLayoutOp::BankRelease`     | F-A1  | Epoch trampoline releases its bank at epoch exit.         |
| `LegalizationOp::FarCall`      | F-A1  | Cross-bank call (slice → expert kernel; thunked).         |
| `PreLayoutOp::Yield`           | F-A1  | Slice yield point at `SchedSlice.yield_check` boundary.   |
| `PreLayoutOp::TraceProbe`      | F-A1  | Trace probe insertion at `ObservationPlan`-tagged ops.    |
| `PreLayoutOp::AssertBank`      | F-A1  | Static assertion that current ROM bank == declared.       |
| `Db { bytes }`                 | F-A1  | Tensor payload bytes for expert sections.                 |
| `Dw { words }`                 | F-A1  | 16-bit lookup tables.                                     |

The codegen pipeline does **not** invent any new pseudo-op. If a
new pattern requires a new pseudo-op, the addition lands in F-A1
first.

The pseudo-ops are emitted via F-A1's `Builder` API (per F-A1 §3):

```rust
builder.bank_lease(spec)?;            // PreLayoutOp::BankLease
builder.bank_release(lease_id)?;      // PreLayoutOp::BankRelease
builder.far_call(bank, target)?;      // LegalizationOp::FarCall
builder.yield_at(yield_kind)?;        // PreLayoutOp::Yield
builder.trace_probe(probe_id)?;       // PreLayoutOp::TraceProbe
builder.assert_bank(rom_bank)?;       // PreLayoutOp::AssertBank
builder.db(bytes)?;                   // Db data directive
builder.dw(words)?;                   // Dw data directive
```

`Builder::validate_effect` rejects `MachineEffect::StoreToMbcRegister`
from `Normal`-class sections at emission time (per F-A1 §4); F-B15
codegen sections are uniformly `Normal`, so any attempt to emit a
raw MBC write fails Builder-locally (no need to wait for the
provenance audit).

### 8.3 Section roles per slice

For each `SchedSlice`, the codegen emits one or more sections with
the following role assignments:

| Slice context                                   | Section role                            | Privilege class             | ExecutionContext (F-A5) |
|------------------------------------------------|----------------------------------------|----------------------------|--------------------------|
| Slice body in common bank kernel               | `CommonKernel`                         | `Normal`                   | `Normal`                 |
| Slice body in expert bank                      | `ExpertPayload(eid)`                   | `Normal`                   | `Normal`                 |
| Slice body in Bank0 (orchestration only)       | `RuntimeBank0`                         | `Normal`                   | `Normal`                 |
| Continuation header (per slice)                | `RuntimeBank0`                         | `Normal`                   | `Normal`                 |
| Mode-level continuation entry                  | `RuntimeBank0`                         | `Normal`                   | `Normal`                 |
| Epoch trampoline                               | `RuntimeBank0`                         | `Normal`                   | `Normal`                 |
| Expert entry stub                              | `ExpertPayload(eid)`                   | `Normal`                   | `Normal`                 |
| Tensor payload                                 | `ExpertPayload(eid)` or `CommonWeights` | `Normal`                   | `Normal`                 |
| Constant data (LUTs, embeddings, classifier)   | `CommonWeights` or `ConstData`         | `Normal`                   | `Normal`                 |
| Trace-only inserts                             | `TraceOnly`                            | `Normal`                   | `Normal`                 |

The codegen front-end never emits sections with `PrivilegeClass::
Privileged`, `PrivilegeClass::InterruptHandler`, or
`SectionRole::IsrReachable`. Those are **F-A5's exclusive territory**.
If a `SchedOp` is emitted into a section that requires privileged
access (e.g. interrupt-mask change), the codegen path takes a
detour through F-A5's exposed runtime helpers (which are
themselves `Privileged`); the calling section stays `Normal`.

### 8.4 Cycle annotations + profile tags

Every emitted `Instr` carries a cycle estimate via F-A1's cycle
model (per F-A1 RFC §6 / `cycle_model.rs`). The cycle estimate is
attached as `InstrProvenance.cycle_before` / `cycle_after`.

Profile tags are attached at the section level. Each section
records its `CompileProfile` (`Bringup` / `Default` / `Trace` /
`Recovery`) so that PlacedRom can apply profile-specific placement
decisions (e.g. `Trace` profile may pin trace-only sections to
SRAM persistent pages with different commit semantics; this is
upstream-decided in F-B11 and consumed here as residency
constraints).

The codegen pipeline does **not** vary AsmIR shape per profile
(per `planv0.md` line 2618: "`ObservabilityMode::Invariant` means
probes must preserve schedule/layout decisions within declared
tolerances"). The profile affects *which sections are emitted*
(e.g. trace-only sections under `Trace`), not *how a given
section is shaped*.

### 8.5 Provenance map

Every emitted `Instr`, `DataBlock`, alignment, label, and
pseudo-op carries provenance pointing back to its origin in the
upstream IRs:

```rust
pub struct AsmIRProvenanceEntry {
    pub section_id: SectionId,
    pub item_index: usize,
    pub source_kind: SourceKind,
}

pub enum SourceKind {
    SchedOp {
        mode: RuntimeMode,
        slice_id: SliceId,
        op_index: usize,
        infer_op: NodeId,         // GbInferIR NodeId
        effect_id: Option<EffectId>,
        checkpoint: Option<SemanticCheckpointId>,
        probe: Option<TraceProbeId>,
    },
    KernelBody {
        kernel_id: KernelSpecId,
        kernel_internal_offset: u32,
    },
    ContinuationHeader {
        mode: RuntimeMode,
        slice_id: SliceId,
        continuation_field: ContinuationField,
    },
    EpochTrampoline {
        mode: RuntimeMode,
        epoch_id: EpochId,
        phase: TrampolinePhase,
    },
    ExpertEntryStub {
        expert_id: ExpertId,
    },
    TensorPayload {
        tensor_id: TensorId,
        offset: u64,
    },
    LutPayload {
        lut_id: LutId,
        offset: u64,
    },
    RuntimeNucleus {
        module: RuntimeShellModule,
    },
    CartridgeHeader,
    BuildIdentityBlock,
}
```

The provenance map is canonically serializable and round-trips
through serde. A property test in
`gbf-codegen::backend::asmir::tests` regenerates the provenance
map from the emitted sections + the original `SchedulePack` and
asserts byte-equality.

The map is consumed by:

* `placed_rom_plan.json` (§10.7) — for the `provenance_summary`
  field per section.
* `reachability_report.json` (§9.5) — for naming the slice/op
  origin of every `(SectionId, item_index)` pair classified.
* `compiler_feedback.json` (F-F1's territory) — for downstream
  reasoning about which slices contributed which placement
  outcomes.
* `gbf-debug` (F-A8) — the agent debugger correlates breakpoints
  back to `SchedSlice` ids.

### 8.6 Self-consistency rules

The AsmIR sub-pass enforces five self-consistency rules at codegen
time. Violations are `ASM-*` diagnostics (§13.1).

#### 8.6.1 Every `SchedOp` is emitted exactly once

For every `(mode, slice_id, op_index)` tuple in `SchedulePack`,
the provenance map contains at least one entry with
`SourceKind::SchedOp` matching that tuple. Multiple AsmIR items
per `SchedOp` are allowed (e.g. a multi-`Instr` lowering); zero
items per `SchedOp` is `ASM-OP-MISSING` (§13.1.6).

#### 8.6.2 Every emitted `MachineEffect::StoreToMbcRegister` has
banking provenance

The codegen pipeline emits no raw MBC writes (§2.1). Every
`StoreToMbcRegister` in the emitted sections must have provenance
pointing to `gbf-runtime::banking` (i.e. a banking helper that
F-A4's `BankingPreLayoutLowering` lowered into the section).
F-A4's `mbc_write_provenance_audit` runs as part of §0a item 6's
closure gate.

#### 8.6.3 Every `BankLease` is balanced

Per F-A4, every `PreLayoutOp::BankLease` must have a matching
`PreLayoutOp::BankRelease`. F-A1's `Builder::finish` (extended by
F-A4) returns `Err(BuilderError::UnreleasedBankGuard)` on
imbalance; F-B15 propagates as `ASM-LEASE-IMBALANCE` (§13.1.7).

#### 8.6.4 Every `FarCall` resolves to a known target

For every `LegalizationOp::FarCall { bank, target }`, the
`target` symbol must resolve to a known `Section` (a kernel, an
expert entry stub, or a runtime helper). Unresolved targets are
`ASM-FARCALL-UNRESOLVED` (§13.1.8).

#### 8.6.5 Every `SemanticCheckpointId` referenced is in the schema

For every `SchedOp` whose provenance carries a
`SemanticCheckpointId`, the id must appear in
`SchedulePack.checkpoint_schema_hash` (i.e. in the
`SemanticCheckpointSchema` whose hash is recorded). Codegen does
not invent checkpoints; it only emits `Yield` / `TraceProbe`
references whose ids the upstream observation pass declared.
Unknown ids are `ASM-CHECKPOINT-UNKNOWN` (§13.1.9).

### 8.7 Cross-bead ownership

The boundary between F-A1 (AsmIR types) and F-B15 (codegen
front-end) is sharp. To re-state it from §1.6:

* **F-A1 owns**: `Instr` and operand subtypes; `Section`,
  `SectionRole`, `OrderedItem<T>`, `LoweredSection`,
  `LegalizedSection`; `MachineEffect`, `MachineEffectKind`;
  `PrivilegeClass`, `SectionPrivilege`; `Builder` and its
  validation rules; `PreLayoutOp`, `LegalizationOp`,
  `BankLeaseSpec`, `LeaseId`, `MbcBankClass`, `YieldKind`,
  `TraceProbeId`, `ProbeLevel`; cycle model; layout pass; relax
  /legalization fixed-point; encoder; ROM builder;
  `.sym`/`.lst` emitters; `SymbolName` and constructors.
* **F-B15 owns**: the `asmir_codegen_core` function signature and
  body; the section group shape per slice (§8.1.1–§8.1.6); the
  pseudo-op selection rules (§8.2); the section role assignment
  (§8.3); the cycle/profile annotation policy (§8.4); the
  provenance map shape (§8.5); the self-consistency rules (§8.6).

If a future bead wants to extend AsmIR (new `Instr` variant, new
`PreLayoutOp` variant, new `SectionRole` variant), the bead lands
in `gbf-asm` and amends F-A1's RFC. F-B15 then consumes the
extension. The reverse (F-B15 RFC adding to AsmIR) is a layering
violation and is forbidden.

### 8.8 The slice-to-AsmIR lowering rules

This subsection is the operational heart of §8: how a single
`SchedSlice` becomes a section group. The rules are
implementation-defined in the sense of §5.4 — they may evolve
without amending this RFC, provided the contract in §8.1–§8.6
holds.

#### 8.8.1 Slice header

Every slice section begins with a tiny header:

```text
slice.<mode>.<slice_id>.entry:
    ; SectionRole = (CommonKernel | ExpertPayload(eid))
    ; PrivilegeClass = Normal
    ; ExecutionContext = Normal
    ; provenance: SchedOp { mode, slice_id, op_index=0, ... }

    AssertBank(rom_bank)             ; PreLayoutOp::AssertBank, declared
                                      ; ROM bank for the section's
                                      ; residency epoch.

    ; Liveness bump (per F-A3 LivenessCounters):
    LD HL, (LIVENESS_PROGRESS_EPOCH)
    INC HL
    LD (LIVENESS_PROGRESS_EPOCH), HL

    ; Slice body follows.
```

The `AssertBank` is a no-op at runtime if the assertion holds; it
is a typed regression at lowering time (F-A4 will reject a slice
section emitted into a bank that does not match the
`AssertBank`'s declared bank).

The liveness bump is a four-instruction sequence per slice entry.
It can be elided when the upstream `ObservationPlan` declares the
slice contains no semantic-progress checkpoint — a property
preserved through `SchedSlice.ops`'s presence/absence of a
`SchedOp::SemanticCheckpoint`.

#### 8.8.2 Op lowering by SchedOp variant

Per F-B13's `SchedOp` enum (consumed by hash), F-B15 lowers each
variant per the following table:

| SchedOp variant                        | Lowering shape                                                                              |
|----------------------------------------|---------------------------------------------------------------------------------------------|
| `SchedOp::CallKernel { kernel_id, .. }` | If kernel residency == current epoch's bank: in-bank `CALL <kernel.<kid>.entry>`. Otherwise: `LegalizationOp::FarCall { bank, target }` (lowered to thunk during PlacedRom). |
| `SchedOp::CallExpert { expert_id, .. }` | Always far-call: `LegalizationOp::FarCall { bank: <expert_bank(eid)>, target: <expert.<eid>.entry> }`. |
| `SchedOp::LoadOperand { ... }`         | A panel/copy sequence into WRAM scratch. Uses `Instr::LdAFromDirect` + `Instr::LdHlFromImm` patterns. |
| `SchedOp::StoreOutput { ... }`         | A panel/copy sequence from WRAM scratch to harness/SRAM/output region. Uses F-A4's `lease_sram` + helper for SRAM. |
| `SchedOp::Yield { kind }`              | `PreLayoutOp::Yield { kind }`. |
| `SchedOp::TraceProbe { probe_id }`     | `PreLayoutOp::TraceProbe(probe_id)`. |
| `SchedOp::SemanticCheckpoint { id }`   | A `Db` directive recording `CompactCheckpointId` + a liveness bump. |
| `SchedOp::AcquireOverlay { overlay_id }` | F-B11's overlay-installer call (the installer is itself an F-A5 nucleus helper, called via in-bank `CALL`). |
| `SchedOp::ReleaseOverlay { overlay_id }` | F-B11's overlay-deinstaller call. |
| `SchedOp::AcquireSequenceState { page_id }` | F-A4's `lease_sram` + load to WRAM working tile. |
| `SchedOp::CommitSequenceState { page_id, group }` | A page-write back through F-A4's `lease_sram`, with the `CommitGroupId` group manifest written last per F-D1's persistent record protocol. |
| `SchedOp::FaultAssert { code }`        | A `Yield { kind: Fault }` with `FaultCode` set in the continuation. |

This table is illustrative; the actual `SchedOp` enum is owned by
F-B13. F-B15 implements one lowering rule per variant. Adding a
variant to `SchedOp` requires both an F-B13 amendment and an
F-B15 lowering addition.

#### 8.8.3 Lowering of cooperative yield checks

`PreLayoutOp::Yield { kind }` lowers (during F-A1's relax) to a
sequence per F-A5's `emit_yield_check`:

```text
    LDH A, (HRAM_LDH_YIELD_REQUESTED)   ; F-A5-owned HRAM byte at $FF84
    AND A
    JR Z, .no_yield                     ; relative jump; in-range under
                                        ; F-A1's branch-relax pass.
    JP <slice.<mode>.<slice_id>.continuation>
.no_yield:
    ; ... fall through into next op.
```

The `JR Z` is in-range when the continuation header sits within
`±127` bytes of the yield check; otherwise F-A1's relax pass
rewrites it to `JP NZ + JP`. F-B15 does not handle relax itself;
it emits the `JR` and trusts the relax pass.

The `<slice_id>.continuation` symbol resolves to a
Bank0-resident continuation section with a typed
`gbf_abi::continuation::ContinuationFrame` writeback. Per F-A3
§3.2, the frame carries `slice_id`, `liveness`, register
snapshot, and the `last_fault` field.

#### 8.8.4 Far-call materialization

For every `SchedOp::CallExpert` or cross-bank
`SchedOp::CallKernel`, codegen emits a single
`LegalizationOp::FarCall { bank, target }`. The legalization op
is **placement-dependent**: the actual thunk emission happens
during PlacedRom (§10.2.2) when bank assignments are known.

Codegen itself does not emit the thunk body; codegen only
emits the *reference*. PlacedRom's invocation of F-A4's
`BankingPreLayoutLowering` produces the per-symbol thunk and the
`CALL <thunk-symbol>` rewrite at the call site.

#### 8.8.5 Kernel binding

For every `SchedOp::CallKernel { kernel_id }`, codegen looks up the
kernel section in `KernelRegistry` keyed by
`(kernel_id, target_residency)` where `target_residency` is the
`KernelResidency` resolved upstream by F-B10's `RomWindowPlan`.
The looked-up section is appended (by reference, not by copy) to
the AsmIR bundle's `codegen_sections` list.

If the kernel section is not in the registry, codegen emits
`ASM-KERNEL-NOT-FOUND` (§13.1.5) with the kernel id and the
expected residency.

#### 8.8.6 Tensor payload emission

For every `SchedOp::LoadOperand { tensor_id, .. }` whose source
is a `TensorId` resolved to an `ExpertPayload` or `CommonWeights`
section, codegen emits the tensor's bytes as a `Db` directive in
the appropriate section. Tensor bytes are read from
`TargetDataLoweringArtifact` (which `ResolvedCompilePolicy` pins
by hash); F-B15 copies them through into AsmIR with provenance
`SourceKind::TensorPayload`.

The actual byte layout per `QuantSpec` (ternary bitplanes,
per-row scales, etc.) is owned by `gbf-artifact::lowerings`. F-B15
treats lowered bytes as opaque blobs from the codegen's
perspective; the `Db` directive carries them through.

Per `planv0.md` line 2537, raw byte blobs are "limited to the
cartridge header and a few tightly audited escape hatches."
Tensor payloads are emitted via `Db`/`Dw` (which carry
provenance) — not via `Raw(Vec<u8>)` (which is structurally
absent from F-A1's `Section` per F-A1 Rule 10 override). This
means tensor payloads count as *audited* by virtue of their
provenance trail to a `TensorId` in the lowering manifest.

#### 8.8.7 LUT emission

Lookup tables (post-norm rescale tables, decode tables, gate
tables) are owned by `gbf-kernel`'s LUT-generator (per
`planv0.md` §"Engineering rules"). F-B15 copies their bytes
through as `Db`/`Dw` directives in `CommonWeights` sections,
with provenance `SourceKind::LutPayload { lut_id, offset }`.

#### 8.8.8 Continuation header shape

Per F-A3 §3.2, the continuation section receives the
`ContinuationFrame` writeback when the slice yields. The header
section shape is:

```text
slice.<mode>.<slice_id>.continuation:
    ; SectionRole = RuntimeBank0
    ; PrivilegeClass = Normal
    ; ExecutionContext = Normal

    ; Save register state to ContinuationFrame in WRAM:
    PUSH AF
    PUSH BC
    PUSH DE
    PUSH HL
    LD A, <slice_id_lo>
    LD (CONTINUATION_FRAME + offset_of(slice_id)), A
    LD A, <slice_id_hi>
    LD (CONTINUATION_FRAME + offset_of(slice_id) + 1), A

    ; Update LivenessCounters:
    ; (no-op if no progress event in this slice)

    ; Return to scheduler entry:
    JP <runtime.scheduler.return_to_kernel>
```

The exact byte sequence is implementation-defined within §5.4's
pluggable surface. The shape — register save, slice-id record,
return — is fixed.

### 8.9 The AsmIRBundle product

The product type is detailed in §2.10. Two derived properties
are worth pinning:

#### 8.9.1 AsmIRBundle is content-addressed

```text
asmir_bundle_hash = DomainHash(
    crate = "gbf-codegen",
    type  = "AsmIRBundle",
    schema_id = "asmir.v1",
    schema_version = "1.0.0",
    canonical_json = canonicalize(AsmIRBundle without report_envelope),
)
```

The `canonicalize` step serializes the bundle with sorted
sections (lex-sorted by canonical SymbolName), sorted maps,
declared item ordering, and sorted provenance entries.

The hash is the `K12` cache key payload (§12.2).

#### 8.9.2 AsmIRBundle is encoder-ready

After codegen, the bundle's sections are in `Section` form
(F-A1's pre-lowered section). They are **not yet** `LegalizedSection`s:
PlacedRom (§10) is what runs the layout/relax/legalization
fixed-point that produces `LegalizedSection`s.

This means a downstream consumer wanting to "see what AsmIR was
generated" can read the bundle's `Section`s directly without
needing to wait for placement. The `asmir_summary.json` (debug-
only, gated behind a `cfg` flag in `gbf-codegen`) serializes the
bundle for debugging.

### 8.10 The AsmIR report body

```rust
pub struct AsmIRReportBody {
    pub asmir_bundle_hash: Hash256,
    pub section_count: u32,
    pub instr_count: u64,
    pub byte_count_estimate: u64,    // pre-relax estimate
    pub bank_lease_count: u32,       // count of PreLayoutOp::BankLease
    pub far_call_count: u32,         // count of LegalizationOp::FarCall
    pub yield_count: u32,            // count of PreLayoutOp::Yield
    pub trace_probe_count: u32,      // count of PreLayoutOp::TraceProbe
    pub kernel_bindings: Vec<KernelBindingSummary>,
    pub provenance_summary: AsmIRProvenanceSummary,
}

pub struct KernelBindingSummary {
    pub kernel_id: KernelSpecId,
    pub residency: KernelResidency,
    pub call_sites: u32,
    pub bytes: u32,
}

pub struct AsmIRProvenanceSummary {
    pub schedop_count: u64,
    pub kernel_body_count: u64,
    pub tensor_payload_count: u32,
    pub lut_payload_count: u32,
    pub runtime_nucleus_section_count: u32,
}
```

The report body is canonical-JSON-serializable. The byte counts
are estimates because final byte counts depend on placement-time
relaxations; final counts live in `placed_rom_plan.json`.

## 9. Sub-pass 2: ReachabilityValidation

### 9.1 The reachability class lattice

ReachabilityValidation classifies every byte of code/data into one
or more reachability classes. The classes form a join-semilattice
under set union; a byte's class is a `ReachabilityClassSet` (a
bit-set over the six base classes).

```rust
pub enum ReachabilityClass {
    /// Reachable from at least one interrupt vector.
    /// MUST satisfy: residency in {Bank0, HRAM, FixedWram}.
    /// MUST NOT have: any `MachineEffect::StoreToMbcRegister`,
    /// any `InterruptControl` other than the F-A4-defined
    /// short-critical-section pattern, any switchable-bank
    /// dependency.
    IsrReachable,

    /// Reachable from a yield-resume continuation entry. After a
    /// `PreLayoutOp::Yield`, the runtime returns to a per-mode
    /// continuation entry; that entry walks the slice list and
    /// dispatches into the appropriate slice.<slice_id>.continuation
    /// section. Every section reachable from there (including the
    /// continuation itself, the dispatcher, and any helpers) is
    /// `YieldResumeReachable`.
    /// MUST satisfy: residency in {Bank0, HRAM, FixedWram}.
    /// (Switchable-bank state is forbidden because the bank
    /// shadow at resume time is not guaranteed to match the
    /// hardware bank without an explicit lease re-acquire.)
    YieldResumeReachable,

    /// Reachable from a fault entry: any path that runs after a
    /// `FaultCode` is raised. Includes the panic section, the
    /// fault-handler dispatcher, any RecoveryAction code, and any
    /// `FaultPolicy`-driven fallback.
    /// MUST satisfy: residency in {Bank0, HRAM, FixedWram}.
    /// (Same reason: the bank shadow may be wrong at fault time.)
    FaultPathReachable,

    /// Reachable from the harness command/result block dispatch.
    /// Harness commands run in the runtime nucleus and may inject
    /// faults, dump arenas, or step slices.
    /// MUST satisfy: residency in {Bank0, HRAM, FixedWram} for
    /// the dispatch surface; harness commands that intentionally
    /// cross banks (e.g. DumpArena targeting an expert bank) MUST
    /// go through F-A4's lease ABI like any other cross-bank
    /// operation.
    HarnessEntryReachable,

    /// Reachable only via a path that holds a BankLease. A
    /// section in this class may safely depend on switchable
    /// state because the lease guarantees the bank is mounted.
    BankLeaseProtected,

    /// Reachable only via "normal" paths (kernel main loop, slice
    /// body, expert call) and NOT reachable from any of the above
    /// privileged contexts.
    NormalOnly,
}

pub struct ReachabilityClassSet {
    bits: u8,           // bit 0 = IsrReachable, ..., bit 5 = NormalOnly
}
```

The lattice ordering:

```text
       T  (top: every class)
      ___|_______________________
     |   |   |   |       |      |
    Isr  Yld  Flt Hns    BLP    Norm
     \   |   |   |       |     /
      \_____\_______/____|____/
                     |
                     ⊥ (bottom: no class — unreachable)
```

The lattice is **not** a partial order with intuitive
"privileged > normal" meaning. Each class names a *property the
byte has*. A byte may be `IsrReachable` and `BankLeaseProtected`
simultaneously when an ISR-reachable section sits inside a
bank-leased epoch. A byte may be `NormalOnly` (no privileged
class) without being unreachable.

The bottom element ⊥ (no class set) is **dead code** — code
emitted but unreachable from any root. Dead code is a
diagnostic class (§9.3.6) but not a hard reject (it bloats the
ROM but does not break correctness). The build emits a
warning-class diagnostic and proceeds.

### 9.2 Edge graph construction

#### 9.2.1 Edges

The edge graph nodes are `(SectionId, item_index)` pairs (or just
`SectionId` for fall-through). Edges are typed:

```rust
pub enum EdgeKind {
    /// Direct in-bank `CALL <symbol>`.
    Call { target: SectionId, return_via: SectionId },

    /// Relative jump (`JR`, `JR cc`).
    JumpRelative { target: SectionId },

    /// Absolute in-bank jump (`JP`, `JP cc`, `JP HL`).
    JumpAbsolute { target: SectionId },

    /// Cross-bank call lowered through a Bank-0 thunk. After
    /// thunk insertion, this edge has four logical legs:
    ///   caller -> thunk
    ///   thunk  -> callee (in target bank)
    ///   callee -> thunk (return)
    ///   thunk  -> caller (return)
    /// The emitted reachability edge rows model the forward
    /// classification legs: the rewritten caller -> thunk
    /// instruction is a normal `Call`/jump edge, and
    /// `FarCallViaThunk` denotes the thunk -> callee leg. The two
    /// return legs document control-flow semantics but do not seed
    /// new forward reachability classes.
    FarCallViaThunk { thunk: SectionId, callee: SectionId,
                       return_via: SectionId },

    /// Fall-through to next section in placement order. Computed
    /// after PlacedRom assigns adjacency. (For pre-placement
    /// reachability, fall-through is computed against codegen's
    /// declared "next slice" relationship.)
    FallThrough { next: SectionId },

    /// Interrupt return (RETI) — back to the pre-interrupt PC.
    /// Modeled as a "return-to-any-IsrReachable-caller" edge for
    /// class propagation purposes.
    InterruptReturn,

    /// Panic jump — irreversible transition into the panic
    /// section.
    PanicJump { target: SectionId },

    /// RST vector call. RST $00, $08, ..., $38 are 1-byte calls
    /// to fixed addresses. F-B15 emits no RST; F-A5 may use them
    /// for ISR dispatch.
    RstVector { vector: u8, target: SectionId },

    /// Indirect jump via a known dispatch table. The table is a
    /// `Db`/`Dw` directive in the originating section; the walker
    /// reads the table's contents and produces one edge per
    /// table entry.
    IndirectJump { dispatch_table: SectionId,
                    targets: BTreeSet<SectionId> },
}
```

Edges are produced by walking the `LegalizedSection`s' instruction
streams (after PlacedRom's relax pass; the reachability walker
re-runs with the legalized graph for the certificate, but a
*pre-placement* topology-only walk is what produces the bank-
independent class assignments — see §2.7 for the ordering
discussion).

The walker produces two reachability reports:

* **Pre-placement topology** — runs after AsmIR codegen, before
  PlacedRom. Computes `IsrReachable`, `YieldResumeReachable`,
  `FaultPathReachable`, `HarnessEntryReachable`, `NormalOnly`
  classes. It may also seed `BankLeaseProtected` from symbolic
  lease regions already present in AsmIR; this seed is a
  lease-boundary fact, not a placement-profile inference.
* **Post-placement final** — runs after PlacedRom, before
  EncodedRom. Carries the pre-placement classes, merges any
  placement-discovered lease facts, and verifies the seven hard
  rules (§9.3) on the legalized graph.

Per §2.7, this is one reachability sub-pass with two phases. The
typed product `ReachabilityReport` carries both the pre-placement
and post-placement class assignments.

For RFC simplicity, when this section says "the reachability
walker runs," it means the unified two-phase walker.

#### 9.2.2 Roots

The five reachability root families:

```rust
pub struct ReachabilityRoots {
    pub interrupt_vectors: BTreeMap<InterruptSource, SectionId>,
    pub harness_entry: SectionId,
    pub continuation_entries: BTreeMap<RuntimeMode, SectionId>,
    pub fault_entries: BTreeMap<FaultDomain, SectionId>,
    pub panic_entry: SectionId,
}
```

* **Interrupt vectors**: per `gbf_hw::interrupts::INT_VECTOR_*`,
  each at a fixed Pan-Docs address (`$0040`, `$0048`, `$0050`,
  `$0058`, `$0060`). Per F-A5, each vector is a typed section
  emitted by F-A5 (§3A.4 of the F-A5 RFC). The reachability
  walker enumerates the five vectors as `IsrReachable` roots.
  The bead `bd-3s0s` ("Make interrupt vectors
  ReachabilityValidation roots") is the F-A1 work that pins the
  vector sections at fixed addresses.
* **Harness entry**: the F-A5 nucleus section that polls the
  `HarnessCommandBlock` doorbell and dispatches commands. Per
  F-A5, the harness polling is part of the cooperative
  scheduler's main loop in M0. The reachability walker
  enumerates the harness-polling section as a
  `HarnessEntryReachable` root.
* **Continuation entries**: per `RuntimeMode`, the mode-level
  continuation entry section emitted by F-B15 (§8.1.2). Each is
  a `YieldResumeReachable` root. F-B15 emits one per mode keyed
  under `SchedulePack.modes`.
* **Fault entries**: per `FaultDomain` (per F-A3 §3.4), the
  fault-handler section that runs when the corresponding
  `FaultCode` is raised. F-A5's panic section is the catch-all
  fault entry for M0; F-D5's `FaultPolicy`, when it lands, will
  introduce per-domain fault entries. Each is a
  `FaultPathReachable` root.
* **Panic entry**: F-A5's `panic` section. A
  `FaultPathReachable` root and a special-cased
  `PanicReachable` (collapsed into `FaultPathReachable` for the
  lattice; the panic-bypass-to-VRAM exemption is recognized by
  the `ExecutionContext::PanicOnly` annotation per F-A5 §3.3).

The walker enumerates roots by reading
`AsmIRBundle.nucleus_sections` and `AsmIRBundle.codegen_sections`
plus the F-A2-defined `INT_VECTOR_*` constants. The enumeration
is deterministic.

Production callers MUST provide the real root set described above.
An implementation may offer an empty-input fallback for harnesses,
fixtures, or narrow-v1 compatibility, but that fallback is not a
production proof of root completeness.

#### 9.2.3 Walker algorithm

The walker is a forward-flow analysis on the typed edge graph:

```text
merge pre-placement seed facts and root facts into class_set
initialize work_queue with every section that owns at least one
seed/root fact
while work_queue not empty:
    node = dequeue
    for each outgoing edge (node, edge_kind, target):
        for each class in node.class_set:
            propagated_class = propagate(class, edge_kind, target)
            if propagated_class not in target.class_set:
                target.class_set ∪= propagated_class
                enqueue target
```

Equivalently, for each individual root class:

```text
for each root in roots:
    initialize work_queue with (root, root_class) pair
    while work_queue not empty:
        (node, class) = dequeue
        if class already in node.class_set: continue
        node.class_set ∪= class
        for each outgoing edge (node, edge_kind, target):
            propagated_class = propagate(class, edge_kind, target)
            enqueue (target, propagated_class)
```

The `propagate` function preserves the class with two
exceptions:

* When traversing an `EdgeKind::FarCallViaThunk` from
  `IsrReachable` source, the target is rejected at validation
  time (§9.3.4) — but if the validator chose to permit it, the
  callee inherits `IsrReachable`. (This is an academic case; the
  ISR rule rejects far-calls from ISRs.)
* When traversing an `EdgeKind::FarCallViaThunk` whose call site
  is inside a `BankLease`-protected region, the callee inherits
  `BankLeaseProtected` (in addition to whatever class the caller
  had).

The walker terminates because the lattice is finite-height (six
elements per node) and every iteration strictly grows at least
one node's class set. Worst-case complexity is O(|nodes| · |edges|
· 6) — linear in graph size for fixed-height lattice.

The walker is implemented as a typed analysis, not a simulation
(§9.4). It does not "execute" the program; it follows the typed
control-flow edges and class-propagation rules.

### 9.3 The seven validation rules

This is the operational core of ReachabilityValidation. The seven
rules are listed in `planv0.md` line 1934+ and are restated here
with their typed enforcement:

#### 9.3.1 Rule 1: ISR-reachable code/data is Bank0/HRAM/fixed-WRAM only

For every `(SectionId, item_index)` whose `ReachabilityClassSet`
contains `IsrReachable`, the section's residency must be
`Residency::Bank0` or `Residency::Hram` or
`Residency::FixedWram`. Switchable ROM, switchable WRAM (CGB
only; ignored on DMG), and SRAM are forbidden.

The rule is enforced on **data** as well as code: an ISR-reachable
section may only `LD A, (HL)` or `LD A, (nn)` from addresses
classified by `gbf_hw::memory::is_isr_resident_legal_dmg(addr)` as
permitted. The walker verifies this by inspecting every memory-
read `MachineEffect` whose address is statically resolvable.

Rejection: `REACH-ISR-BANK-DEPENDENCY` (§13.2.1).

`Cite`: `planv0.md` line 121, line 1936, line 1993.

#### 9.3.2 Rule 2: No forbidden MBC writes on privileged paths

For every section reachable from any `IsrReachable` /
`YieldResumeReachable` / `FaultPathReachable` /
`HarnessEntryReachable` root that does **not** sit behind a
`BankLease`-protected boundary, no instruction may have
`MachineEffect::StoreToMbcRegister`. (The walker enforces
"reachable but not lease-protected" by checking that every path
from a privileged root to the offending instruction has at least
one path that does not pass through a lease acquire.)

The rule is essentially: "if your path is privileged and
unlocked, you must not change the bank." Since lease acquires
themselves are MBC writes, and since lease-protected regions
are by definition "we have permission to bank," the rule
collapses to: **no raw bank-changing writes on privileged
paths**. F-A4's `BankingPreLayoutLowering` is the only legal
producer; if anything in the lowered sections has
`StoreToMbcRegister` provenance pointing outside `gbf-runtime::
banking`, the audit (§2.1) caught it earlier — so this rule is
near-redundant in practice. It remains as a structural
defense-in-depth check.

Rejection: `REACH-PRIVILEGED-MBC-WRITE` (§13.2.2).

`Cite`: `planv0.md` line 1937.

#### 9.3.3 Rule 3: No illegal MachineEffect on PrivilegeClass-forbidden paths

Per F-A1 §4, `PrivilegeClass` declares which
`MachineEffect`s a section may emit:

| PrivilegeClass        | Allowed MachineEffects                                                 |
|----------------------|------------------------------------------------------------------------|
| `Normal`             | All except `StoreToMbcRegister`, `InterruptControl`, `OamAccess`*, `VramAccess`*. |
| `Privileged`         | `Normal` plus `StoreToMbcRegister`, `RamEnableWrite`.                 |
| `InterruptHandler`   | `Privileged` plus the F-A4-blessed short-critical-section pattern.    |

(`*` exception: `VideoCommitOnly` execution-context sections may emit
`VramAccess` and `OamAccess`. The `panic` section's `PanicBypass`
is the audit-recognized exemption per F-A5 §3.3.)

The rule: every `(SectionId, item_index)`'s emitted
`MachineEffect`s must be in the allowed set for the section's
declared `PrivilegeClass`. F-A1's `Builder::validate_effect`
enforces this at emission time, so this rule is essentially
already enforced by codegen. ReachabilityValidation cross-checks
at validation time as a defense-in-depth.

Rejection: `REACH-PRIVILEGE-VIOLATION` (§13.2.3).

`Cite`: `planv0.md` line 1938.

#### 9.3.4 Rule 4: No switchable-bank dependency on ISR or resume paths

For every `(SectionId, item_index)` whose
`ReachabilityClassSet` contains `IsrReachable` or
`YieldResumeReachable`, no edge may target a section whose
residency is switchable. Cross-bank calls from ISR/resume paths
are forbidden (even via thunks — the thunk itself runs in
Bank0, but the callee is in switchable space).

The rule subsumes Rule 1 for code paths and extends it to data
references: an ISR-reachable LUT load must not target a
switchable-bank LUT.

Rejection: `REACH-PRIVILEGED-SWITCHABLE-DEPENDENCY` (§13.2.4).

`Cite`: `planv0.md` line 1939.

#### 9.3.5 Rule 5: No illegal reentrancy through bank guards

A `BankLease` is acquired and released in a balanced fashion
(F-A4's invariant). The reachability walker enforces:

* No edge from inside a lease region back to the same lease's
  acquire site (would re-enter the lease without releasing).
* No edge from inside a lease region into a nested acquire of
  an overlapping bank without explicit release of the outer
  lease (would create stacked banks; F-A4 does not support
  `LeaseLifetime::ResumeWindow` in M2 closure scope).
* No yield (`PreLayoutOp::Yield`) inside a lease region whose
  declared `yield_safe == false`.

These checks build on F-B13's `ResourceStateValidation`
certificate (`certs/resource_state.cert.json`). F-B15's
contribution is verifying that the *legalized* graph (after
far-call thunk insertion) preserves the resource-state proof.
F-B13's proof is on the pre-thunk graph; thunk insertion can in
principle introduce reentrancy if a thunk's body recursively
calls into itself, so F-B15 re-verifies after legalization.

Rejection: `REACH-LEASE-REENTRANCY` (§13.2.5).

`Cite`: `planv0.md` line 1940.

#### 9.3.6 Rule 6: No unreachable continuation targets

Every `slice.<mode>.<slice_id>.continuation` section must be
reachable from at least one `YieldResumeReachable` root.
(Specifically, from the per-`RuntimeMode` continuation entry's
dispatcher.)

The dual: every `JP <continuation>` in a slice's yield path must
target a section that the walker has classified
`YieldResumeReachable`. If not, the yield falls through to
unclassified code (i.e. the runtime returns to unknown
territory).

Dead-code in this rule is **not** unreachable continuations —
those are a hard reject. Dead-code is unreachable codegen
sections (e.g. a kernel that no `SchedOp::CallKernel` references).

Rejection (continuation): `REACH-CONTINUATION-UNREACHABLE` (§13.2.8).
Warning (dead code): `REACH-DEAD-CODE` (§13.2.9, severity Soft;
F-B15 promotes to Hard if `CompileObjective.no_dead_code` is set).

`Cite`: `planv0.md` line 1941.

#### 9.3.7 Rule 7: No fault path that depends on non-resident data

Every `FaultPathReachable` section's data reads must target
addresses in `Bank0`, `Hram`, or `FixedWram` — i.e. the same
residency rule as ISR. The reasoning is identical: at fault
time, the bank shadow may be wrong, and re-acquiring banks is
itself a privileged operation that may not always be safe.

The rule excludes one specific class: explicitly fault-recoverable
banked state. F-D5's `FaultPolicy` may declare that certain
recovery actions take a `BankLease` before reading recovery data;
those actions are `BankLeaseProtected` *and* `FaultPathReachable`,
and the rule's data-read predicate respects the lease-protection.

Rejection: `REACH-FAULT-PATH-NONRESIDENT-DATA` (§13.2.10).

`Cite`: `planv0.md` line 1942.

### 9.4 Decision procedure

The decision procedure is a **typed analysis**, not a simulation.
The walker:

1. Builds the typed edge graph (§9.2.1) from the
   `LegalizedSection` instruction streams plus a static
   resolution of indirect jumps (§9.2.1's `EdgeKind::IndirectJump`).
2. Enumerates roots (§9.2.2).
3. Performs a forward-flow class propagation (§9.2.3) until
   convergence.
4. Verifies the seven rules (§9.3) against the converged
   classification.
5. Emits findings (`ReachabilityFinding` per §9.5).

The walker does **not**:

* Execute the program (no instruction simulation).
* Symbolically interpret register values (only statically-
  resolvable address operands are handled; dynamic-address ops
  are flagged with their `LoadFromDynamic { via }` /
  `StoreToDynamic { via }` annotations from F-A1's effect
  classifier; the walker treats them conservatively as
  potentially touching any address).
* Compute path predicates or path coverage. The lattice is
  per-node; the rules are per-node.

This typed-analysis posture matches `planv0.md` line 1944 and is
the only correct posture: simulating LR35902 to validate banking
would conflate the validation pass with `gbf-emu`. The two are
orthogonal: `gbf-emu` runs the encoded ROM at simulation time;
ReachabilityValidation proves the encoded ROM's *typing* before
it ever runs.

The walker terminates in O(|nodes| · |edges|) time per the
lattice height argument in §9.2.3.

### 9.5 Certificate shape — `reachability_cert.v1`

```rust
pub struct ReachabilityCertificate {
    pub schema_id: ReportSchemaId,            // "reachability_cert.v1"
    pub schema_version: SchemaVersion,        // "1.0.0"
    pub identity: ReachabilityCertIdentity,
    pub roots: ReachabilityRoots,
    pub class_summary: ReachabilityClassSummary,
    pub findings: Vec<ReachabilityFinding>,
    pub validator_witness_hash: Hash256,
    pub cert_self_hash: Hash256,
}

pub struct ReachabilityClassSummary {
    pub by_class: BTreeMap<ReachabilityClass, ClassStats>,
    pub dead_code_byte_count: u32,
    pub dead_code_section_count: u32,
}

pub struct ClassStats {
    pub section_count: u32,
    pub byte_count: u32,
    pub representative_sections: Vec<SectionId>,  // up to 16
}

pub struct ReachabilityFinding {
    pub rule: RuleId,
    pub status: FindingStatus,            // Holds | Violated
    pub code: ValidationCode,
    pub witness: FindingWitness,          // path / addresses
}

pub enum RuleId {
    R1_IsrResidency,
    R2_PrivilegedMbcWrite,
    R3_PrivilegeViolation,
    R4_PrivilegedSwitchable,
    R5_LeaseReentrancy,
    R6_ContinuationReachable,
    R7_FaultPathResidency,
}

pub enum FindingStatus {
    Holds,
    Violated,
}

pub enum FindingWitness {
    PathFromRoot {
        root: ReachabilityRoot,
        path: Vec<(SectionId, usize)>,
    },
    DataAddress {
        section: SectionId,
        offset: usize,
        bad_address: u16,
        residency: Residency,
    },
    LeaseImbalance {
        acquire_at: (SectionId, usize),
        release_at: Option<(SectionId, usize)>,
    },
    NoWitness,           // for `Holds` findings; the absence
                         // of a counterexample is the witness.
}
```

The certificate is emitted only when **all seven** rules hold.
A failed validation produces `reachability_report.json` with
`outcome: Failed` and a list of `Violated` findings; no
certificate is produced.

The `validator_witness_hash` is a hash of a serialized
sub-structure that an independent walker (in `gbf-verify`, owned
by F-F2) can use to verify the certificate. The witness includes:

* The full edge graph (compressed canonical form).
* The class assignment per node.
* For each rule, a proof obligation discharger:
  * For `Holds` findings, the witness is "no counterexample
    found" plus the search exhaustiveness proof (the walker's
    convergence trace).
  * For dead-code summary, the unreachable-set computed.

The cross-validator in `gbf-verify` runs an independent forward-
flow analysis using a reference implementation; if its results
agree with F-B15's certificate, the validation passes. The
two-implementation cross-check is the load-bearing safety
property for the certificate.

### 9.6 Cross-cutting check: ResourceStateValidation reconciliation

F-B13's `ResourceStateValidation` pass (Stage 10.5) emits
`certs/resource_state.cert.json` with **annotation-driven**
proofs:

* Lease balance (every `AcquireLease` has a matching
  `ReleaseLease` per slice).
* No illegal yield across non-resumable lease.
* No ISR-visible path depends on leased switchable state.
* Overlay-shadow assumptions match declared residency.

These proofs use **declared** annotations on slices, ops, and
leases. F-B15's ReachabilityValidation, by contrast, uses
**computed** classes derived from the typed edge graph.

The two passes prove related-but-distinct things:

| Property                                  | F-B13 (annotation-driven) | F-B15 (computed) |
|-------------------------------------------|--------------------------|------------------|
| Lease balance per slice                   | Yes                      | Yes (defense-in-depth) |
| Yield-safety per slice                    | Yes                      | Yes (defense-in-depth) |
| ISR-vs-leased-state per declaration       | Yes                      | No (out of scope) |
| ISR-vs-leased-state per call graph        | No                       | **Yes** (Rule 4) |
| Cross-bank thunk reentrancy               | No                       | **Yes** (Rule 5) |
| Continuation reachability                 | No                       | **Yes** (Rule 6) |
| Privileged-path data-residency            | No                       | **Yes** (Rule 7) |
| Privilege-class effect compliance         | F-A1 Builder enforces    | Yes (defense-in-depth, Rule 3) |

Where the two passes overlap (lease balance, yield-safety), F-B15
re-runs the check on the legalized graph as defense-in-depth.
Where they disagree, **F-B15 wins**, and F-B15 emits a
diagnostic that F-B13's annotations were wrong:

* `REACH-CLASS-DISAGREEMENT` (§13.2.7) — F-B15 computed a class
  that disagrees with F-B13's `ResourceVector` annotation for
  the same slice. The diagnostic names both classes and the path
  that produced the disagreement. F-B16, when it lands, may turn
  this into a `RepairProposal::AnnotationCorrection`; F-B15
  itself rejects the build.

The reconciliation is **one-directional**: F-B15 may reject what
F-B13 accepted (a stricter computation revealed an unsafe path),
but F-B15 does not accept what F-B13 rejected (since F-B13 ran
first and its rejection prevents F-B15 from running at all). The
net effect is that F-B13's pass is a fast-path filter; F-B15's
pass is the final word.

This is the safety property that catches "the hard cases" F-A4
RFC's Decision 5 names: "the hard cases (ISR transitively
reaches a privileged banking helper through a long call chain)
are declared here and *proved* later by Epic B's
`ReachabilityValidation`. F-A4 provides the declaration
substrate, not the global proof." The "global proof" is §9.

### 9.7 The reachability report body

```rust
pub struct ReachabilityReportBody {
    pub class_per_section: BTreeMap<SectionId, ReachabilityClassSet>,
    pub class_per_byte_summary: ReachabilityByteSummary,
    pub edge_graph_hash: Hash256,
    pub roots: ReachabilityRoots,
    pub findings: Vec<ReachabilityFinding>,
    pub disagreements_with_resource_state: Vec<ClassDisagreement>,
    pub dead_code: DeadCodeSummary,
    pub walker_version: SchemaVersion,
}

pub struct ReachabilityByteSummary {
    pub total_bytes: u32,
    pub by_class: BTreeMap<ReachabilityClass, u32>,
    // class_per_byte detail is in the certificate, not the
    // human-facing report (would be too verbose).
}

pub struct ClassDisagreement {
    pub section: SectionId,
    pub fb13_declared: ResourceClassSet,
    pub fb15_computed: ReachabilityClassSet,
    pub witness_path: Vec<(SectionId, usize)>,
}

pub struct DeadCodeSummary {
    pub section_count: u32,
    pub byte_count: u32,
    pub representatives: Vec<SectionId>,    // up to 16
}
```

The report's `outcome` is `Passed` iff every finding is `Holds`
and `disagreements_with_resource_state` is empty. Dead code is
reported as a Soft diagnostic (promoted to Hard when
`CompileObjective.no_dead_code` is set per §9.3.6).

### 9.8 What the reachability pass does NOT do

* It does **not** verify that the encoded ROM's bytes match the
  PlacedRom decisions. That is a property of the encoder
  (§11.4) and is checked by `ENC-DRIFT` (§13.4.1).
* It does **not** verify cycle budgets or interrupt-latency
  budgets. Those are F-B14 / F-B13 territory.
* It does **not** verify continuation-frame layout. F-A3's
  `static_assertions::const_assert_eq!` calls at compile time
  ensure the layout. F-B15's reachability sub-pass only
  classifies the byte ranges; it does not introspect the
  frame's typed contents.
* It does **not** verify VRAM/OAM access discipline. F-A5's
  `video_commit` is the sole writer; the audit walk that
  verifies single-writer is in F-A5's closure gates (§4.6 of
  F-A5 RFC). F-B15 trusts that audit by hash of the F-A5
  nucleus.
* It does **not** verify persistent-state commit ordering. F-D1
  (persistence) owns that. F-B15 verifies that
  `SchedOp::CommitSequenceState` lowers to a `lease_sram` +
  page-write + commit-manifest sequence (§8.8.2), but the
  group-commit-last invariant is upstream-proven.
* It does **not** verify trace-event budgeting. F-D3 owns the
  trace ring + drop policy. F-B15 verifies that
  `PreLayoutOp::TraceProbe` lowers to a typed trace-event
  enqueue (§8.8.2 row).
* It does **not** verify oracle correspondence. F-C2/F-C3 own
  that.

### 9.9 Performance notes

The walker is bounded by `O(N · E · K)` where `N` is the number
of sections, `E` is the number of edges per section, and `K=6`
is the lattice height. For a typical M2 build (one shared
common-bank kernel + 8 expert payloads + 4 modes), N ≈ 60
sections, E ≈ 8 edges per section, K = 6 ⇒ ≈ 3000 work units.
The walker runs in well under 1 second on a developer machine.

The certificate emission produces a compressed witness data
structure on the order of 100 KB for a typical M2 build.
`gbf-verify`'s independent walker re-validates the certificate
in similar time.

## 10. Sub-pass 3: PlacedRom

PlacedRom is the heart of the backend: it takes the AsmIR
sections + the reachability classification and produces a
concrete bank assignment, byte address, and legalized instruction
stream for every section.

### 10.1 Placement profiles

PlacedRom invokes a profile-specific layout strategy. The three
profiles are typed in `gbf-policy::PlacementProfile` and consumed
by F-B15:

#### 10.1.1 `PlacementProfile::StrictOnePerBank`

Bring-up / debug profile. Rules:

* Each `SectionRole::ExpertPayload(ExpertId)` section group is
  placed alone in its own bank. No co-residency of multiple
  experts in one bank.
* `SectionRole::CommonKernel` sections may share a single
  common bank.
* `SectionRole::CommonWeights` sections (embeddings, classifier
  head, router weights) may share a separate common bank.
* `SectionRole::Bank0Nucleus` (F-A5) goes in Bank 0.
* No bank packing optimization. Slack is ignored as long as the
  sections fit individually.

Use case: bring-up debugging, where bank-packing bugs are
expensive to diagnose. The default for `Bringup` profile per
`planv0.md` line 2585.

If a single expert exceeds 16 KiB, `StrictOnePerBank` fails with
`PLACE-EXPERT-TOO-LARGE` (§13.3.1) — there is no co-residency
to fall back to.

#### 10.1.2 `PlacementProfile::Budgeted`

Default profile. Rules:

* Each expert section group still goes in its own bank, but
  with a declared **slack budget** (default 25% of bank size,
  configurable via `RuntimeChromeBudget.expert_bank_slack_pct`).
* Common-bank kernels may pack into a single common bank as
  long as total bytes ≤ bank size − slack.
* Common-weights sections pack into one or more common-weight
  banks per the same packing rule.
* Bank 0 packs the F-A5 nucleus + far-call thunks + (optionally)
  hot expert kernels per `RomWindowPlan`'s
  `KernelResidency::Bank0Fixed` directive.
* Bank-switch coalescing is enabled (§10.2.6).
* Hotness-driven placement is enabled: experts whose
  `ExportFacts.expert_hotness` is high are placed in
  lower-numbered banks (closer to Bank 0 in the MBC5 numbering
  scheme; this matters for some emulator and hardware-specific
  bank-switch latency models). The hotness ordering is
  upstream-pinned in `ExportFacts`; F-B15 consumes it
  deterministically.

Use case: production builds. The default for `Default` profile.

`Budgeted` may **fail** with `PLACE-EXPERT-COMMON-BANK-PRESSURE`
(§13.3.2) when the common-bank packing exceeds capacity. F-B16's
loop driver (when it lands) may then promote to `PackedExperts`.

#### 10.1.3 `PlacementProfile::PackedExperts`

Tight profile. Rules:

* Same as `Budgeted` plus: multiple small experts may
  co-reside in one bank when:
  1. Their combined size ≤ bank size − slack.
  2. Their reachability classes are compatible (no mixing of
     `IsrReachable` experts with `NormalOnly` experts in the
     same bank).
  3. They share a `RomWindowPlan` window assignment (i.e. the
     scheduler can mount the bank once and call into multiple
     experts without an intervening lease release).
* Bank-switch coalescing is more aggressive (§10.2.6).
* The packing decision is **deterministic**: experts are
  assigned to banks in canonical order (sorted by
  `ExpertId`, then by hotness). The first bank that fits the
  next expert receives it. This is a first-fit-decreasing
  variant; the determinism is the key property, not the bin-
  packing optimality.

Use case: M3+ builds with tight bank counts and multiple small
experts. Used by `Recovery` profile per `planv0.md` line 2598+.

`PackedExperts` may still fail with
`PLACE-PACKED-INFEASIBLE` (§13.3.7) when no co-residency
arrangement satisfies the rules. The diagnostic names which
experts collided.

#### 10.1.4 Profile is selected, not derived

The profile is read from `ResolvedCompilePolicy.placement_profile`.
F-B15 does **not** auto-promote profiles. F-B16's loop driver,
when it lands, does the promotion by re-invoking F-B15 with a
new policy.

The profile is recorded in `placed_rom_plan.json.placement_profile`
and echoed in `map.json.placement_profile`. K14's StageCache key
includes the profile (§12.4) — different profiles produce different
PlacedRoms, even with identical AsmIR.

### 10.2 Layout / legalization sub-passes

PlacedRom runs nine sub-sub-passes in strict order. Each sub-sub-
pass uses F-A1's pre-existing layout/relax/legalization
machinery; F-B15 supplies the policy-driven inputs and aggregates
the results.

#### 10.2.1 Bank assignment

The bank assignment phase distributes sections to banks per the
selected profile (§10.1). Output: `BTreeMap<SectionId, BankIndex>`.

The phase is deterministic. Algorithm sketch (Budgeted):

```text
1. Place all SectionRole::Bank0Nucleus sections in Bank 0.
2. Compute total Bank 0 free space after nucleus + reserved
   thunks pool.
3. For each `KernelResidency::Bank0Fixed` kernel from RomWindowPlan,
   place in Bank 0. If exceeds free space, fail with
   PLACE-BANK0-PRESSURE (§13.3.3).
4. Compute common-kernel bank set: place CommonKernel sections
   in canonical-sorted order, packing into bank N+1 where N is
   the count of expert+common banks already placed. First-fit-
   decreasing into common-kernel banks.
5. Compute common-weights bank set: pack CommonWeights sections
   similarly into common-weight banks.
6. Compute expert bank set: each ExpertPayload(eid) group goes
   alone in its own bank (Budgeted) or packs (PackedExperts).
   Hotness-ordered (high hotness ⇒ lower bank number).
7. Verify: sum of bank usage ≤ TargetProfile.cartridge_profile
   .rom_size.bytes(). If violated, fail with PLACE-ROM-OVERFLOW
   (§13.3.4).
```

The algorithm is documented in detail in
`gbf-codegen::backend::placed::strategy` and is reference-
testable against fixtures.

#### 10.2.2 Far-call thunk insertion

For every `LegalizationOp::FarCall { bank, target }` in the
AsmIR, PlacedRom invokes F-A4's `BankingPreLayoutLowering` to
emit a thunk (§2.6). The thunk:

* Lives in Bank 0.
* Acquires `ValidatedBankLeaseSpec { bank: target_bank, ... }`
  via `lease_rom_switchable`.
* Calls into the target symbol (`CALL <target>`).
* Releases the lease via `release_bank` on return.
* Carries `InterruptPolicy::ShortCriticalSection` discipline
  per F-A4 §4.

Thunks are deduplicated per `(target_bank, target_symbol)`: one
thunk per pair, regardless of how many call sites reference it.
The thunk's symbol is `runtime.banking.thunk.<target_symbol>`.
Per-call-site rewrite changes `LegalizationOp::FarCall` into
`Instr::CallDirect { target: <thunk_symbol> }`.

The thunk pool is reserved in Bank 0 at a specific address range
declared by F-A1 (`ROM0_THUNK_POOL_START` per the F-A1 layout
constants). PlacedRom places thunks in canonical order (sorted
by target symbol) starting at the pool address.

#### 10.2.3 Branch relaxation

After thunk insertion, F-A1's `gbf-asm::relax::relax_and_legalize`
runs the iterative-monotone fixed-point pass per F-A1 §5. The
pass:

* Widens out-of-range `JR` to `JP NZ + JP` (or similar).
* Inserts fall-through `NOP`s where required for in-range
  branches.
* Iterates until no widening is needed.

The pass is owned by F-A1; F-B15 invokes it once per
PlacedRom. The pass terminates because it is monotone (each
iteration only widens, never shrinks).

If the pass fails (e.g. an out-of-range `JR` whose target lies
outside the section's bank and cannot be rewritten), PlacedRom
fails with `PLACE-RELAX-FAILED` (§13.3.5).

#### 10.2.4 Stable symbol naming

After bank assignment + thunk insertion + relax, every section
has a final SymbolName. The naming scheme:

```text
runtime.<module>.<entry>            # F-A5 nucleus modules
runtime.banking.thunk.<target>      # F-A4 thunks
slice.<mode>.<slice_id>.entry       # codegen slice entries
slice.<mode>.<slice_id>.continuation # codegen continuations
mode.<mode>.continuation_entry      # mode-level entry
epoch.<mode>.<epoch_id>.trampoline  # epoch trampolines
expert.<expert_id>.entry            # expert entry stubs
expert.<expert_id>.payload.<idx>    # expert tensor sub-payloads
common_kernel.<kernel_id>.entry     # shared kernel entries
common_weights.<tensor_id>          # shared weight tables
const_data.<lut_id>                 # LUT/constant tables
trace_only.<probe_id>               # trace-only sections
cartridge.header                    # cartridge header
build_identity_block                # the BuildIdentityBlock
vector.<source>                     # interrupt vectors (F-A5-emitted)
```

The naming is fully deterministic. Symbols are sorted in `.sym`
output by their canonical name lex order.

#### 10.2.5 Residency enforcement

PlacedRom enforces residency rules at bank-assignment time:

* `KernelResidency::Bank0Fixed` ⇒ section MUST be in Bank 0.
* `KernelResidency::CommonBank(BankId)` ⇒ section MUST be in
  the named common bank.
* `KernelResidency::ExpertBank(ExpertId)` ⇒ section MUST be in
  the expert's bank. (One bank per ExpertId in
  StrictOnePerBank/Budgeted; possibly co-resident in
  PackedExperts.)
* `KernelResidency::WramOverlay(OverlayId)` ⇒ section MUST be
  staged into a WRAM overlay region per F-B11's
  `OverlayPlan`. The section's "primary residence" (the
  source bytes) is in some ROM bank; the overlay is its
  runtime residence.

The **ISR residency rule** is enforced against
ReachabilityValidation's computed classification: every section
whose `ReachabilityClassSet` contains `IsrReachable` MUST be
assigned to Bank 0 (or, for data, HRAM/FixedWram). If the
upstream `RomWindowPlan` resolved a kernel as
`KernelResidency::CommonBank(_)` but reachability marks it
`IsrReachable`, PlacedRom rejects the build with
`PLACE-ISR-NON-BANK0` (§13.3.10) and emits a diagnostic that
F-B10's plan is wrong.

Per §2.3, F-B15 wins when computation disagrees with declaration.

#### 10.2.6 Bank-switch coalescing

For consecutive `LegalizationOp::FarCall { bank: B }` calls into
the same target bank, the relax pass coalesces them: the lease
is acquired once before the first call and released once after
the last call (provided the calls are consecutive in slice
order and no intervening `Yield` or other lease-impacting op
sits between them).

Coalescing is enabled in `Budgeted` and `PackedExperts`,
disabled in `StrictOnePerBank` (where each call gets its own
acquire/release pair to make bank-switch sequences explicit for
debugging).

Coalescing reduces the bank-switch count per token, which feeds
back into F-B14's cost report. F-B15 records the coalescing
count in `placed_rom_plan.json.coalesced_bank_switches` and the
post-coalescing bank-switch count in
`map.json.bank_switches_per_token`.

#### 10.2.7 Deterministic section ordering

Within each bank, sections are placed in canonical order:

```text
1. SectionRole order:
   Bank0Nucleus < BankingThunk < CommonKernel < CommonWeights
   < ExpertPayload(eid) < ConstData < TraceOnly
   (For cross-bank, this defines the ordering across bank
   boundaries; within a bank, it defines the placement order
   inside the bank.)

2. Within a SectionRole, sections are sorted by canonical
   SymbolName.

3. Tied symbols (impossible if naming is deterministic) are
   broken by source provenance hash.
```

The ordering is documented in
`gbf-codegen::backend::placed::ordering` and is property-tested
against fixtures.

#### 10.2.8 Common-bank vs expert-bank partitioning

The partitioning rule:

* `SectionRole::CommonKernel` and `SectionRole::CommonWeights`
  go in **common banks** — banks that may be mounted alongside
  one expert. Common banks are reserved for "shared by every
  expert" data.
* `SectionRole::ExpertPayload(ExpertId)` goes in **expert
  banks** — one bank per expert (Budgeted/StrictOnePerBank) or
  shared banks per the packing rule (PackedExperts).

The partitioning matches the runtime architecture from
`planv0.md` line 2076+: "ROM banks 01..K — CommonBanks", "ROM
banks K+1..N — ExpertBanks". F-B15 enforces this partition
deterministically.

Common banks are mounted by the scheduler before entering an
expert call, then the expert bank is mounted on top of (or
instead of) the common bank, depending on whether the expert
needs both. The lease-acquire ordering is part of the
slice-to-AsmIR codegen (§8.8.2) — F-B15 here only verifies that
the partition is respected.

#### 10.2.9 Continuation-target validity check

Per §10.6 (cross-references F-B13 continuation contracts), every
`JP <continuation>` in a slice's yield path MUST target a
section that:

* Is reachable from the corresponding `RuntimeMode`'s
  continuation entry (verified by ReachabilityValidation Rule
  6, §9.3.6).
* Is in-bank: the continuation section is Bank0-resident, and
  the yield's `JP` is from a slice section that may be in a
  switchable bank — but the `JP` is rewritten to a far-jump
  via the relax pass (or the yield path explicitly returns to
  Bank 0 first). F-B15 verifies that no `JP <continuation>`
  is left as an in-bank jump from a switchable section.

Violation: `PLACE-CONTINUATION-CROSS-BANK` (§13.3.11).

### 10.3 Common-bank vs expert-bank partitioning (detailed)

This section expands §10.2.8.

The partitioning is computed once per build:

```text
let common_banks = banks(0..=K);            // Bank 0 + K common banks
let expert_banks = banks(K+1..=N);          // N-K expert banks

where:
  K = ceil(common_kernel_bytes / bank_size_bytes)
    + ceil(common_weights_bytes / bank_size_bytes)
  N = K + expert_count (Budgeted)
    or
    K + ceil(expert_total_bytes / bank_size_bytes)  (PackedExperts)

  bank_size_bytes = 16 KiB (MBC5 switchable window).
```

Bank 0 is treated specially: it hosts the F-A5 nucleus,
banking thunks, and (per `KernelResidency::Bank0Fixed`) hot
kernels that need to stream expert data. Bank 0's free space
after these placements is reserved for codegen-emitted
mode-level continuation entries, epoch trampolines, and
slice continuation headers.

If `K + N > rom_size_in_banks`, the build fails with
`PLACE-ROM-OVERFLOW` (§13.3.4) and F-B16 may propose
profile promotion (`Budgeted → PackedExperts`).

### 10.4 Bank packing determinism (canonical ordering proof)

Determinism is a top-level property (§2.8). For PlacedRom
specifically, determinism means: same AsmIR + same TargetProfile
+ same PlacementProfile + same RomWindowPlan + same
OverlayPlan + same ArenaPlan ⇒ same bank assignment + same
section ordering + same legalized byte sequence.

The canonical-ordering proof has three parts:

1. **Bank assignment is deterministic.** The algorithms in
   §10.2.1 are first-fit (Budgeted) or first-fit-decreasing
   (PackedExperts). Both are deterministic given a sorted
   input. Inputs are sorted by `ExpertId` (then by hotness,
   then by canonical symbol name).

2. **Section ordering within a bank is deterministic.** Per
   §10.2.7's lex-by-role-then-by-symbol ordering.

3. **Thunk pool is deterministic.** Thunks are sorted by
   target symbol; the pool address is fixed by F-A1.

A property test in `gbf-codegen::backend::placed::tests`
regenerates a PlacedRom twice on the same fixture and asserts:

* `bank_assignments` is byte-equal.
* `legalized` (the legalized section list) is element-wise
  `LegalizedSection`-equal.
* `symbol_table` is byte-equal.
* `placed_rom_self_hash` is byte-equal.

The test is part of the closure gate for §0a item 1.

### 10.5 Global constraints

PlacedRom enforces six global constraints. Violations are
`PLACE-*` diagnostics (§13.3).

#### 10.5.1 No section crosses a bank boundary

Each `LegalizedSection`'s byte range fits entirely within one
16 KiB bank window (`$0000..=$3FFF` for Bank 0; `$4000..=$7FFF`
for switchable). A section that exceeds the bank window is
rejected with `PLACE-SECTION-CROSSES-BANK` (§13.3.12).

The constraint applies to **post-relax** sections. A section
whose pre-relax size fit in a bank but whose post-relax size
exceeds it (e.g. due to many `JR → JP` widenings) is rejected
at the same diagnostic class.

`Cite`: `planv0.md` line 1968.

#### 10.5.2 All relative branches in range or rewritten

After F-A1's relax pass converges, every `Instr::Jr` and
`Instr::JrCc` has its target within `±127` bytes of the
post-instruction PC. Any `JR` that cannot be relaxed (target
outside the section's bank) is rejected with
`PLACE-RELAX-FAILED` (§13.3.5).

`Cite`: `planv0.md` line 1969.

#### 10.5.3 All expert sections satisfy residency rules

For every `SectionRole::ExpertPayload(eid)` section, the
section's bank assignment must match the expert's
`KernelResidency` from `RomWindowPlan` (or the policy-resolved
override). Mismatch is `PLACE-EXPERT-RESIDENCY` (§13.3.13).

The cross-check with `ReachabilityValidation`'s computed
classes (Rule 1, ISR residency) is §10.2.5.

`Cite`: `planv0.md` line 1970.

#### 10.5.4 All SRAM/WRAM arenas fit

Per F-B11/F-B12's `ArenaPlan`, every arena slot has a
declared byte range. PlacedRom verifies:

* `WramHotArena`: total bytes ≤ 8 KiB minus declared overlay
  reservations from `OverlayPlan`.
* `WramOverlay`: total bytes ≤ overlay-region budget per
  `OverlayPlan`.
* `Hram`: total bytes ≤ 127 minus F-A4 banking-shadow bytes
  ($FF80..=$FF83) minus F-A5-owned yield-requested byte
  ($FF84) minus declared scheduler/text/keyboard private
  bytes.
* `Sram`: total bytes ≤ 8 KiB per page; persistent pages must
  fit the SRAM bank count declared by `TargetProfile`.

Overflow is `PLACE-ARENA-OVERFLOW-{WRAM,HRAM,SRAM}`
(§13.3.14).

`Cite`: `planv0.md` line 1971.

#### 10.5.5 All continuation targets valid + reachable

Per §10.2.9 + §9.3.6, every `JP <continuation>` resolves to a
section that ReachabilityValidation marks
`YieldResumeReachable`. Violation: redundant with §9.3.6 but
re-checked here as defense-in-depth via
`PLACE-CONTINUATION-INVALID` (§13.3.15).

`Cite`: `planv0.md` line 1972.

#### 10.5.6 Bank packing is deterministic

Per §10.4. Violation is detected by the regeneration property
test (`PLACE-NONDETERMINISM`, §13.3.16).

`Cite`: `planv0.md` line 1973.

#### 10.5.7 Future-reservation byte ranges respected

F-A5's `RuntimeShellModule::FutureReservation` declarations
(per F-A5 §1.1.x) reserve byte ranges in Bank 0 for the
deferred runtime modules (`persistence`, `trace`, `harness`).
PlacedRom must not place codegen sections into those ranges.

The constraint is informational (no compiled code touches
those ranges yet), but the reservation is byte-level: the
final `.gb` has those bytes filled with `$FF` (per ROM padding)
or with the F-A5-declared placeholder pattern. Violation is
`PLACE-FUTURE-RESERVATION-COLLISION` (§13.3.17).

`Cite`: `planv0.md` line 1995–2003 (Bank 0 partition).

### 10.6 Continuation-target validity check (detailed)

This sub-pass is the cross-reference between F-B13's
continuation contracts and F-B15's placement decisions.

F-B13 (`bd-9ae`) declares for each slice:

* The yield kind (`SchedSlice.yield_kind`).
* The continuation target (implicit; the per-slice continuation
  section).
* The continuation frame layout (per F-A3 `InferenceState`
  prefix).

F-B15 verifies:

* For every `SchedOp::Yield { kind }` in slice `S`, F-B15 emits
  a `JP <continuation_of_S>` that resolves to
  `slice.<mode>.<slice_id>.continuation`.
* That section is placed in Bank 0 (per §10.2.5 rules, since
  it must be resumed from any bank).
* That section is reachable from the per-mode continuation
  entry's dispatcher (§9.3.6).
* The continuation frame writes (per §8.8.8) write to WRAM
  addresses that match `gbf_abi::continuation::InferenceState`
  prefix layout (per F-A3 §3.2).

Violation: `PLACE-CONTINUATION-INVALID` (§13.3.15) with detail
naming the slice and the missing/wrong target.

### 10.7 `placed_rom_plan.json` schema

```rust
pub struct PlacedRomPlanReportBody {
    pub placement_profile: PlacementProfile,
    pub bank_count_used: u16,
    pub bank_assignments: BTreeMap<SectionId, BankIndex>,
    pub section_summaries: Vec<PlacedSectionSummary>,
    pub thunk_summaries: Vec<ThunkSummary>,
    pub coalesced_bank_switches: u32,
    pub layout_iterations: u32,           // F-A1 relax count
    pub global_constraints: GlobalConstraintsSummary,
    pub residency_enforcement: ResidencyEnforcementSummary,
    pub partitioning: PartitioningSummary,
    pub layout_algorithm_version: SchemaVersion,
}

pub struct PlacedSectionSummary {
    pub section_id: SectionId,
    pub symbol: SymbolName,
    pub role: SectionRole,
    pub privilege: PrivilegeClass,
    pub bank: BankIndex,
    pub start_addr: u16,
    pub size_bytes: u32,
    pub residency: Residency,
    pub reachability_classes: ReachabilityClassSet,
    pub origin: PlacedSectionOrigin,
    pub legalization_steps: Vec<LegalizationStep>,
    // Cycle estimate carried through from F-A1's cycle model:
    pub cycles_min: Option<CycleBudget>,
    pub cycles_max: Option<CycleBudget>,
}

pub enum PlacedSectionOrigin {
    SchedSlice { mode: RuntimeMode, slice_id: SliceId },
    EpochTrampoline { mode: RuntimeMode, epoch_id: EpochId },
    ContinuationEntry { mode: RuntimeMode },
    ExpertEntry { expert_id: ExpertId },
    ExpertPayload { expert_id: ExpertId, segment: u16 },
    CommonKernel { kernel_id: KernelSpecId },
    CommonWeights { tensor_id: TensorId },
    LutPayload { lut_id: LutId },
    RuntimeNucleus { module: RuntimeShellModule },
    BankingThunk { target: SymbolName },
    CartridgeHeader,
    BuildIdentityBlock,
    InterruptVector { source: InterruptSource },
}

pub struct ThunkSummary {
    pub symbol: SymbolName,                  // runtime.banking.thunk.X
    pub target_bank: BankIndex,
    pub target_symbol: SymbolName,
    pub call_sites: Vec<(SectionId, usize)>,
    pub bytes: u32,
}

pub struct GlobalConstraintsSummary {
    pub no_section_crosses_bank: bool,
    pub all_branches_in_range: bool,
    pub all_expert_residency_satisfied: bool,
    pub all_arenas_fit: bool,
    pub all_continuations_valid: bool,
    pub bank_packing_deterministic: bool,    // by regeneration test
}

pub struct LegalizationStep {
    pub kind: LegalizationStepKind,
    pub before_bytes: u32,
    pub after_bytes: u32,
}

pub enum LegalizationStepKind {
    BranchRelax { from: InstrId, to: InstrId },
    FarCallThunked { thunk: SymbolName },
    BankSwitchCoalesce { count: u32 },
    AlignmentInsert { bytes: u32 },
}
```

### 10.8 `map.json` schema

`map.json` is the load-bearing build artifact for the rest of
the toolchain. Its schema is:

```rust
pub struct MapReportBody {
    pub schema_id: ReportSchemaId,         // "map.v1"
    pub schema_version: SchemaVersion,     // "1.0.0"
    pub identity: MapIdentity,
    pub placement_profile: PlacementProfile,
    pub rom_size_bytes: u32,
    pub rom_size_banks: u16,
    pub bank_summaries: BTreeMap<BankIndex, BankSummary>,
    pub address_map: BTreeMap<u32, MapEntry>,    // absolute address keyed
    pub symbol_index: BTreeMap<SymbolName, u32>, // symbol -> address
    pub arena_map: ArenaMap,
    pub vector_map: BTreeMap<InterruptSource, u16>,
    pub build_identity_block_addr: u16,
    pub harness_command_block_addr: u16,
    pub harness_result_block_addr: u16,
    pub continuation_frame_addr: u16,
    pub liveness_counters_addr: u16,
    pub trace_ring_addr: u16,
    pub persistent_pages: Vec<PersistentPageEntry>,
    pub cycle_budget: CycleBudgetSummary,        // from F-B14
    pub bank_switches_per_token: BankSwitchSummary,
    pub map_self_hash: Hash256,
}

pub struct BankSummary {
    pub bank: BankIndex,
    pub used_bytes: u32,
    pub free_bytes: u32,
    pub section_count: u16,
    pub partition: BankPartition,
    pub reachability_summary: ReachabilityClassSet,
}

pub enum BankPartition {
    Bank0Nucleus,
    Bank0FreeForCodegen,        // codegen sections in bank 0
    CommonKernel,
    CommonWeights,
    ExpertPayload(Vec<ExpertId>),  // 1 entry for Budgeted; multiple for PackedExperts
}

pub struct ArenaMap {
    pub wram_hot: Vec<ArenaEntry>,
    pub wram_overlay: Vec<ArenaEntry>,
    pub hram: Vec<ArenaEntry>,
    pub sram: Vec<ArenaEntry>,
}

pub struct ArenaEntry {
    pub name: ArenaName,
    pub start: u16,
    pub size_bytes: u32,
    pub lifetime_class: LifetimeClass,
    pub commit_group: Option<CommitGroupId>,
    pub provenance: ArenaProvenance,
}

pub struct PersistentPageEntry {
    pub page_id: PersistPageId,
    pub commit_group: CommitGroupId,
    pub sram_bank: u8,
    pub start: u16,
    pub size_bytes: u32,
    pub durability_class: DurabilityClass,    // F-A3
    pub kind: PersistKind,                    // F-A3
}

pub struct CycleBudgetSummary {
    pub expected_cycles_per_token: BTreeMap<RuntimeMode, u64>,
    pub worst_case_interrupt_latency: u32,    // M-cycles
    pub frame_budget_m_cycles: u32,           // = 17_556 from gbf-hw
    pub utilization_pct: BTreeMap<RuntimeMode, u8>,
}

pub struct BankSwitchSummary {
    pub estimated_per_token: BTreeMap<RuntimeMode, u32>,
    pub coalesced_savings: u32,
}
```

The schema is **the** load-bearing schema for downstream
tooling. Field additions are minor-bumps; field renames or
type changes are major-bumps. Per §7.2.

#### 10.8.1 `map.json` consumers

| Consumer       | What it reads                                                         |
|---------------|-----------------------------------------------------------------------|
| `gbf-debug`    | `symbol_index`, `address_map`, `bank_summaries`, `vector_map`         |
| `gbf-emu`      | `build_identity_block_addr`, `harness_*_addr`, `continuation_frame_addr` |
| `gbf-bench`    | `cycle_budget`, `bank_switches_per_token`, `bank_summaries.used_bytes` |
| `gbf-verify`   | `address_map` (cross-validates against `.lst` and `.gb`)              |
| `gbf-report`   | All — for `build_manifest.json` aggregation                          |
| Harness (F-D2) | `harness_command_block_addr`, `harness_result_block_addr`            |
| Persistence (F-D1) | `persistent_pages` for boot validation                            |

#### 10.8.2 `map.json` self-hash

`map_self_hash` is computed per the F-B2/F-B4 self-hash
convention (§4.1) over the canonical-JSON form with the field
set to `ZERO_HASH`. The hash is over the entire report body
including `address_map` and `symbol_index`.

The hash is the byte-stable identifier for `map.json` and is
the load-bearing handle that downstream tools use to assert
they are looking at the same build's map.

#### 10.8.3 Address space normalization

`address_map` is keyed by `u32` because absolute addresses for
banked sections include a bank prefix:

```text
0x0000..=0x3FFF       Bank 0 ROM (and HRAM, WRAM, IO via the
                      same address prefix; ambiguous; resolved by
                      `MapEntry.address_space`)
0x4000..=0x7FFF       Switchable ROM window (bank-N only when
                      mounted; encoded in MapEntry.bank)
0x8000..=0x9FFF       VRAM
0xA000..=0xBFFF       SRAM window (switchable)
0xC000..=0xDFFF       WRAM
0xFE00..=0xFE9F       OAM
0xFEA0..=0xFEFF       Unmapped (Pan Docs warns)
0xFF00..=0xFF7F       I/O registers
0xFF80..=0xFFFE       HRAM
0xFFFF                IE register
```

For banked addresses, the `u32` key encodes
`(bank_index << 16) | address_low`. Bank 0 entries use
bank_index = 0 and addr in `$0000..=$3FFF`; switchable entries
use bank_index = N and addr in `$4000..=$7FFF`. Other regions
use bank_index = 0.

The encoding is canonical and deterministic.

#### 10.8.4 Cost annotations from F-B14

The `cycle_budget` and `bank_switches_per_token` fields are
populated from F-B14's `ScheduleCostReport.per_mode`. F-B15
does **not** re-compute costs; it copies the F-B14 envelope
through with provenance `MapEntryProvenance::F-B14`.

If F-B14's envelope shows a per-token utilization >100%, F-B15
emits `PLACE-COST-DRIFT` (§13.3.18) — but this is a soft
diagnostic in M2 (F-B16 is not yet shipped to drive a retry).
In M3+, F-B16 will turn this into a `RepairProposal::ReduceTraceDensity` or similar.

## 11. Sub-pass 4: EncodedRom

EncodedRom is intentionally boring. It serializes the PlacedRom
into bytes, the symbol table into `.sym` text, and the
interleaved listing into `.lst` text. Every choice point has
already been made; this sub-pass merely materializes them.

### 11.1 `.gb` shape (cartridge)

The `.gb` file is a Pan-Docs-conformant MBC5 cartridge image:

```text
$0000..=$00FF   RST vectors + Bank0 boot region (F-A5 supplies)
$0040..=$0067   Interrupt vector stubs (F-A5; placed first-class
                per §2.2)
$0100..=$0103   Cartridge entry stub (F-A1's gbf-asm::rom emits)
$0104..=$0133   Nintendo logo (F-A2 NINTENDO_LOGO constant)
$0134..=$0143   Title (CompileRequest.title or default)
$0144..=$0145   New licensee code
$0146           SGB flag (= 0)
$0147           MBC type (CartridgeProfile.mbc_type.header_byte())
$0148           ROM size code (CartridgeProfile.rom_size.header_byte())
$0149           RAM size code (CartridgeProfile.ram_size.header_byte())
$014A           Destination code (CartridgeProfile.destination_code)
$014B           Old licensee code
$014C           Mask ROM version
$014D           Header checksum (F-A1's gbf-asm::rom computes)
$014E..=$014F   Global checksum (F-A1's gbf-asm::rom computes)
$0150..         Runtime boot entry + Bank0 nucleus + thunks +
                codegen Bank0 sections
                ...
                BuildIdentityBlock at F-A5-defined offset
                ...
$3FFF           End of Bank 0
$4000..         Switchable banks: common kernel, common weights,
                expert payloads
                ...
End             ROM total = TargetProfile.rom_size.bytes()
                (32 KiB / 64 KiB / .. / 8 MiB)
```

#### 11.1.1 Cartridge header

The cartridge header is emitted via F-A1's `gbf-asm::rom::
assemble_rom`. F-B15 supplies a `CartridgeHeader` value:

```rust
let header = gbf_asm::rom::CartridgeHeader {
    title: policy.cartridge_title.clone(),
    mbc_type: target.cartridge_profile.mbc_type,
    rom_size: target.cartridge_profile.rom_size,
    ram_size: target.cartridge_profile.ram_size,
    destination_code: target.cartridge_profile.destination_code,
    new_licensee_code: policy.cartridge_new_licensee_code,
    mask_rom_version: policy.cartridge_mask_rom_version,
};
```

F-A1's ROM builder fills the Nintendo logo, computes the header
checksum and global checksum, and emits the bytes at the correct
offsets. F-B15 has no policy here.

#### 11.1.2 Vector placement

Per §2.2, vectors are first-class. EncodedRom serializes them at
their fixed addresses. F-A1's `gbf-asm::layout::layout_into_banks`
honors the pinned addresses (the `LayoutPlan::pin_vector` API);
EncodedRom merely walks the layout and emits the bytes.

#### 11.1.3 Padding

Bytes between sections (within a bank) are padded with `$FF`
(the standard ROM unprogrammed-byte value). Bytes in
`FutureReservation` ranges are padded with the F-A5-declared
placeholder pattern (or `$FF` if no placeholder declared).

The padding is computed by F-A1's encoder; EncodedRom does not
choose the padding byte. The padding byte is documented in F-A5
RFC §3H.3 (normalization rules for `runtime_nucleus_hash`).

#### 11.1.4 BuildIdentityBlock emission

Per §8.1.5, the `BuildIdentityBlock` is emitted at a known
offset by F-A5's boot section. F-B15's role at encoding time:

1. Serialize the block with `build_hash = ZERO_HASH`.
2. Encode the rest of the ROM.
3. Compute `build_hash = SHA256(rom_bytes)` over the full
   ROM with the `build_hash` field set to `ZERO_HASH`.
4. Patch the `build_hash` field in place with the computed
   value.

The "patch in place" step is the single piece of mutation in
EncodedRom. It is `pub(crate)` to `gbf-codegen::backend::encoded`
and verifiable: a property test asserts that recomputing the
hash over the patched ROM (with the `build_hash` field re-zeroed)
matches the patched value.

The other three lineage hashes (`artifact_core_hash`,
`runtime_nucleus_hash`, `compile_request_hash`) are passed in by
upstream stages and embedded as constants — no patch needed.

#### 11.1.5 Total byte size

The total `.gb` byte size equals
`TargetProfile.cartridge_profile.rom_size.bytes()`. If PlacedRom's
total used bytes < ROM size, the trailing region is `$FF`-padded
to the ROM size. If PlacedRom's used bytes > ROM size, that is
`PLACE-ROM-OVERFLOW` (§13.3.4) and EncodedRom never runs.

#### 11.1.6 `.gb` self-hash

After patching the `BuildIdentityBlock.build_hash`, the final
`.gb` byte sequence is hashed:

```text
encoded_rom_self_hash = SHA256(gb_bytes)
```

This hash is the canonical identifier of the build. It appears
in:

* `placed_rom_plan.json.identity.encoded_rom_self_hash` (set
  after EncodedRom runs).
* `map.json.identity.encoded_rom_self_hash`.
* `BuildIdentityBlock.build_hash` (per §11.1.4 — same value, by
  construction).
* `gbf-debug` session files (per F-A8's session ROM identity
  fixation).

### 11.2 `.sym` shape (RGBDS-compatible symbol map)

The `.sym` format is the standard line-oriented Game Boy symbol
format (per F-A1 §10):

```text
BB:AAAA name
```

where `BB` is the bank index in two-digit hex, `AAAA` is the
address within the bank (or absolute address for non-banked
regions) in four-digit hex, and `name` is the canonical
SymbolName from §10.2.4.

Lines are sorted lex by name. Comments are not emitted.

Example excerpt:

```text
00:0040 vector.vblank
00:0048 vector.lcd_stat
00:0050 vector.timer
00:0058 vector.serial
00:0060 vector.joypad
00:0100 cartridge.header
00:0150 runtime.boot.entry
00:0200 runtime.scheduler.main_loop
00:0300 runtime.banking.thunk.expert_0_entry
00:0308 runtime.banking.thunk.expert_1_entry
00:0500 mode.steady_state.continuation_entry
00:0680 epoch.steady_state.epoch_0.trampoline
00:0700 slice.steady_state.slice_0.continuation
00:0780 slice.steady_state.slice_1.continuation
01:4000 common_kernel.matvec_i8.entry
01:4400 common_kernel.norm_rms.entry
02:4000 common_weights.embedding
03:4000 expert.expert_0.entry
03:4040 expert.expert_0.payload.0
04:4000 expert.expert_1.entry
...
```

The `.sym` is consumed by `gbf-debug` (per F-A8 RFC) to power
agent breakpoints, watchpoints, and `gb.symbol(name)` /
`gb.symbol_at(addr)` script calls. The session-file format
embeds the entire `SymbolTable` (per F-A8 RFC's session-file
shape) so rehydration is independent of the `.sym` file's
filesystem path.

#### 11.2.1 Sort stability

Per §10.2.4, symbol names are deterministic. Per §10.4, bank
assignments are deterministic. Per §10.2.7, addresses within a
bank are deterministic. Therefore the `.sym` line ordering is
deterministic.

A property test asserts that two regenerations of the same
fixture produce byte-equal `.sym` files.

#### 11.2.2 `.sym` writer

F-A1's `gbf-asm::symbols::write_sym` is the unique writer.
F-B15 supplies the `SymbolTable` (built up during PlacedRom)
and calls the writer once. The writer is byte-stable per F-A1's
closure tests.

### 11.3 `.lst` shape (interleaved listing)

The `.lst` file is the human-readable interleaved listing per
F-A1 §9. It interleaves disassembled instructions, addresses,
encoded bytes, and originating provenance:

```text
; ============================================================
; SECTION runtime.boot.entry
; SectionRole = RuntimeBank0
; PrivilegeClass = Normal
; ExecutionContext = Normal
; Bank 00 @ $0150..$01A7 (88 bytes)
; Origin: RuntimeNucleus(Boot)
; ============================================================
00:0150  3E 0A             LD   A, $0A           ; provenance: F-A5 boot.rs:14
00:0152  EA 00 00          LD   ($0000), A       ; provenance: F-A5 boot.rs:15 (RAMG enable)
00:0155  CD 00 03          CALL $0300            ; provenance: F-A5 boot.rs:18
                                                  ; -> runtime.banking.thunk.expert_0_entry

; ============================================================
; SECTION slice.steady_state.slice_0.entry
; SectionRole = CommonKernel
; PrivilegeClass = Normal
; ExecutionContext = Normal
; Bank 01 @ $4000..$4080 (128 bytes)
; Origin: SchedSlice { mode: SteadyState, slice_id: 0 }
; ============================================================
01:4000  E5                PUSH HL               ; provenance: SchedOp(SteadyState, 0, op=0)
01:4001  21 80 C0          LD   HL, $C080        ; provenance: SchedOp(SteadyState, 0, op=1) [WRAM panel A]
01:4004  CD 00 41          CALL $4100            ; provenance: SchedOp(SteadyState, 0, op=2)
                                                  ; -> common_kernel.matvec_i8.entry
01:4007  ...
```

The format includes:

* Per-section header with metadata.
* Per-instruction line with `BB:AAAA  bytes  mnemonic  ; provenance`.
* Branch-target comments resolving to symbol names.
* Yield-point markers showing the continuation target.
* Bank-lease enter/exit markers.
* Trace-probe markers.
* Cycle-cost annotations (post-relax) per F-A1 §6 / cycle model.

#### 11.3.1 `.lst` writer

F-A1's `gbf-asm::listing::emit_listing` is the unique writer.
F-B15 supplies the `LayoutPlan` + `LegalizedSection`s + the
provenance map and calls the writer once. The writer is
byte-stable per F-A1's closure tests.

#### 11.3.2 `.lst` consumers

| Consumer       | What it reads                                                 |
|---------------|---------------------------------------------------------------|
| `gbf-debug`    | Listing context for breakpoints (`gb.lst_at(addr)` script)   |
| `gbf-verify`   | Cross-validates against `.gb` byte sequence                  |
| Human review   | Debugging compiled output, performance investigation         |

### 11.4 The encoder is tiny — every byte traces

Per §2.5, the encoder is intentionally tiny. The driver
`run_encoded_rom` is approximately:

```rust
pub fn run_encoded_rom(placed: &PlacedRom)
    -> Result<(EncodedRom, ReportEnvelope<EncodedRomReportBody>),
              PassDiagnostics> {
    // Step 1: assemble the .gb via F-A1's ROM builder.
    let mut gb_bytes = gbf_asm::rom::assemble_rom(
        &placed.layout,
        &placed.legalized,
        placed.cartridge_header.clone(),
    )?;

    // Step 2: patch BuildIdentityBlock.build_hash.
    let build_hash_offset = placed.layout.find_symbol(
        "build_identity_block",
    )?.offset + offset_of_build_hash;
    let zero_hash_at = build_hash_offset..build_hash_offset+32;
    gb_bytes[zero_hash_at.clone()].fill(0);
    let build_hash = sha256(&gb_bytes);
    gb_bytes[zero_hash_at].copy_from_slice(&build_hash);

    // Step 3: write .sym via F-A1.
    let sym_lines = gbf_asm::symbols::write_sym(
        &placed.symbol_table,
    )?;

    // Step 4: write .lst via F-A1.
    let lst_text = gbf_asm::listing::emit_listing(
        &placed.layout,
        &placed.legalized,
        &placed.symbol_table,
        provenance,  // from AsmIRBundle
    )?;

    // Step 5: identity + report envelope.
    let identity = EncodedRomIdentity {
        placed_rom_self_hash: placed.identity.placed_rom_self_hash,
        encoded_rom_self_hash: sha256(&gb_bytes),
        build_hash,
        encoder_version: ENCODER_VERSION,
    };
    let report = ReportEnvelope { ... };

    Ok((EncodedRom { gb_bytes, sym_lines, lst_text, identity,
                     report_envelope: report }, report))
}
```

The driver has **no `match` on `Instr`**, **no symbol resolution
loop**, **no byte tables**. Every byte produced is mediated by
an F-A1 entry point. The only `gbf-codegen`-local logic is the
build-hash patch (§11.1.4) and the report-envelope assembly.

#### 11.4.1 Encoder version

`ENCODER_VERSION` is a constant in `gbf-codegen::backend::encoded`
that bumps when:

* The build-hash patch algorithm changes.
* The byte-ordering convention for sections changes.
* The `.sym` / `.lst` invocation parameters change.

It does **not** bump when F-A1's underlying encoder or symbol/
listing writers change — those bumps live in
`LAYOUT_ALGORITHM_VERSION` (§3.19). The two version constants
are independent.

`ENCODER_VERSION` is part of K15's StageCache key (§12.5).

#### 11.4.2 EncodedRom report body

```rust
pub struct EncodedRomReportBody {
    pub gb_byte_count: u32,
    pub sym_line_count: u32,
    pub lst_byte_count: u32,
    pub build_hash: [u8; 32],          // BuildIdentityBlock.build_hash
    pub encoded_rom_self_hash: Hash256, // SHA256 of full .gb
    pub encoder_version: SchemaVersion,
    pub padding_byte_counts: PaddingSummary,
    pub bank_byte_counts: BTreeMap<BankIndex, u32>,
}

pub struct PaddingSummary {
    pub bank0_padding: u32,
    pub common_bank_padding: u32,
    pub expert_bank_padding: u32,
    pub future_reservation_padding: u32,
}
```

The report is small (the `.gb` itself is the artifact; the
report is metadata). It serves as the integrity record for the
encoded artifact.

### 11.5 What EncodedRom does NOT do

* It does **not** sign or verify the ROM. There is no
  cryptographic signature on the cartridge.
* It does **not** flash the ROM to a cartridge. Flashing is a
  deployment concern owned by `tools/flash/` (per the
  workspace skeleton in `planv0.md` line 180).
* It does **not** emit `.sav` files for SRAM. SRAM is empty at
  cartridge boot; the persistence layer (F-D1) writes to it at
  runtime.
* It does **not** validate the encoded `.gb` against
  `gbf-emu`. That cross-check is a follow-up bead in
  `gbf-test`'s integration matrix.

### 11.6 Byte-stability test

A property test in `gbf-codegen::backend::encoded::tests`:

1. Constructs a fixture build (synthetic SchedulePack +
   minimal nucleus + minimal target profile).
2. Runs `run_stage12` to produce `Stage12Output`.
3. Saves `stage12_output_a = clone(stage12_output)`.
4. Re-runs `run_stage12` with the same inputs.
5. Asserts:
   * `gb_bytes == stage12_output_a.encoded.gb_bytes`
   * `sym_lines == stage12_output_a.encoded.sym_lines`
   * `lst_text == stage12_output_a.encoded.lst_text`
   * `placed_rom_self_hash == stage12_output_a.placed.identity
      .placed_rom_self_hash`
   * `encoded_rom_self_hash == stage12_output_a.encoded.identity
      .encoded_rom_self_hash`
   * Every report's `report_self_hash` is byte-equal across the
     two runs.

The test is gated as part of §0a item 1's closure condition.

## 12. StageCache algebra

F-B15 wires four StageCache keys: K12 (AsmIR), K13
(Reachability), K14 (PlacedRom), K15 (EncodedRom). Each follows
the F-B2/F-B4 §11 key construction rule.

### 12.1 General key shape

Per F-B2/F-B4 §11:

```text
StageCacheKey = DomainHash(
    crate          = "gbf-codegen",
    type_name      = <KeyTypeName>,
    schema_id      = <KeySchemaId>,
    schema_version = <KeySchemaVersion>,
    canonical_json = canonicalize(<key_body>),
)
```

The cache stores keyed entries; on a hit, the cache returns the
prior typed product (the sub-pass output), bypassing the pure
core. On a miss, the driver runs the pure core and writes the
result to the cache.

### 12.2 K12: `AsmIRCacheKey`

```rust
pub struct AsmIRCacheKey {
    pub schedule_pack_hash: Hash256,
    pub resolved_compile_policy_hash: Hash256,
    pub target_profile_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub schedule_cost_report_hash: Hash256,
    pub kernel_registry_hash: Hash256,
    pub asmir_codegen_version: SchemaVersion,
}
```

Cached value: `AsmIRBundle`.

The `kernel_registry_hash` covers the set of kernels the codegen
binds to. A new kernel implementation invalidates K12 even if
the SchedulePack is unchanged.

The `runtime_nucleus_hash` covers the F-A5 nucleus sections.
A nucleus rebuild (e.g. F-A5 panic-section change) invalidates
K12 because codegen references nucleus symbols (continuation
return targets, harness polling helpers).

### 12.3 K13: `ReachabilityCacheKey`

```rust
pub struct ReachabilityCacheKey {
    pub asmir_bundle_hash: Hash256,
    pub reachability_walker_version: SchemaVersion,
}
```

Cached value: `ReachabilityReport`.

The walker version captures algorithm changes; the bundle hash
captures the input edge graph (which is fully determined by the
AsmIR).

K13 does **not** include the placement profile because the
pre-placement topology classification is profile-independent.
The post-placement final-classification phase (§9.2.1's two-
phase walker) is folded into K14: the `BankLeaseProtected`
classification depends on bank assignments, which depend on the
profile.

### 12.4 K14: `PlacedRomCacheKey`

```rust
pub struct PlacedRomCacheKey {
    pub asmir_bundle_hash: Hash256,
    pub reachability_report_hash: Hash256,
    pub placement_profile: PlacementProfile,
    pub layout_algorithm_version: SchemaVersion,
    pub overlay_plan_hash: Hash256,
    pub arena_plan_hash: Hash256,
    pub rom_window_plan_hash: Hash256,
    pub schedule_cost_report_hash: Hash256,  // for cost annotations
}
```

Cached value: `PlacedRom`.

K14 is the most complex key because PlacedRom integrates the
most upstream products: AsmIR + Reachability + the three
spatial plans (Overlay/Arena/RomWindow) + the cost report (for
the budget annotations baked into `map.json`).

A change in any upstream input (or in the layout algorithm)
invalidates K14.

### 12.5 K15: `EncodedRomCacheKey`

```rust
pub struct EncodedRomCacheKey {
    pub placed_rom_self_hash: Hash256,
    pub cartridge_header_hash: Hash256,
    pub build_identity_args_hash: Hash256,
    pub encoder_version: SchemaVersion,
}
```

Cached value: `EncodedRom`.

K15 is small: PlacedRom is the load-bearing input, plus the
cartridge header (which is build-specific) plus the build-
identity args (which include the build timestamp under non-
BitExact determinism).

### 12.6 Cache discipline

* **Cold cache**: a fresh checkout has no entries. The first
  build runs all four sub-passes from scratch.
* **Warm cache**: subsequent builds with identical inputs
  serve from the cache; only the report-emission step runs.
* **Partial cache**: a SchedulePack change invalidates K12,
  which invalidates K13, K14, K15. A target-profile change
  invalidates K12 onward. A placement-profile change
  invalidates only K14 + K15 (K12 and K13 are profile-agnostic).
* **Failure memoization**: per F-B2/F-B4 §11, failures are
  memoized too. A SchedulePack that previously failed AsmIR
  codegen with `ASM-KERNEL-NOT-FOUND` produces an immediate
  cached failure on retry without re-running the codegen pass.

The cache is `gbf-store`'s territory (per F-A6); F-B15 wires
keys but does not implement the cache.

### 12.7 Cache key tests

For each key, F-B15 ships:

* A round-trip test asserting the key serializes/deserializes
  through canonical JSON.
* A perturbation test asserting that changing each field
  produces a distinct key hash.
* A determinism test asserting that two identical inputs
  produce identical key hashes across runs.

The tests are gated as part of §0a item 5's closure condition.

## 13. Diagnostic algebra

F-B15 introduces four diagnostic-code families: `ASM-*`
(codegen), `REACH-*` (reachability), `PLACE-*` (placement),
`ENC-*` (encoding). Each code is a closed enum value with a
typed renderable detail.

Per F-B2/F-B4 §5: codes are closed (`D-CodeClosed`); no string-
only error path (`D-NoStringOnly`); every detail is renderable
(`D-Renderable`); every diagnostic carries provenance
(`D-Provenance`).

### 13.1 ASM-* — AsmIR codegen diagnostics

#### 13.1.1 `ASM-LOWERING-MISSING`

A `SchedOp` variant has no lowering rule in the codegen pipeline.
Severity: Hard.

```rust
ValidationDetail::AsmLoweringMissing {
    mode: RuntimeMode,
    slice_id: SliceId,
    op_index: usize,
    schedop_variant: String,           // for renderable detail only
}
```

Cause: F-B13 introduced a new `SchedOp` variant without an
F-B15 lowering update. Resolution: extend the codegen front-end
(an F-B15 follow-up bead) plus an RFC amendment to §8.8.2's
table.

#### 13.1.2 `ASM-NUCLEUS-COLLISION`

An F-A5-emitted nucleus section's symbol collides with a
codegen-emitted symbol. Severity: Hard.

```rust
ValidationDetail::AsmNucleusCollision {
    nucleus_section: SectionId,
    codegen_section: SectionId,
    symbol: SymbolName,
}
```

Cause: codegen used a reserved name. The naming scheme in
§10.2.4 is exclusive; codegen must not invent names that overlap
with `runtime.<module>.<entry>` or `runtime.banking.thunk.<X>`
patterns.

#### 13.1.3 `ASM-MBC-RAW-WRITE`

A codegen-emitted instruction has `MachineEffect::
StoreToMbcRegister` outside of an F-A4 lowering path.
Severity: Hard.

```rust
ValidationDetail::AsmMbcRawWrite {
    section: SectionId,
    item_index: usize,
    instruction: String,
    target_register: u16,
}
```

Cause: codegen emitted a raw MBC write. This is structurally
forbidden by F-A1's `Builder::validate_effect`; reaching this
diagnostic means a defect in either codegen (bypassing the
Builder) or F-A4's lowering (emitting a section without
provenance).

#### 13.1.4 `ASM-MBC-PROVENANCE-VIOLATION`

A `MachineEffect::StoreToMbcRegister` instruction has provenance
pointing outside `gbf-runtime::banking`. Severity: Hard.

```rust
ValidationDetail::AsmMbcProvenanceViolation {
    section: SectionId,
    item_index: usize,
    expected_origin: String,           // "gbf-runtime::banking"
    actual_origin: String,
}
```

Cause: F-A4's `mbc_write_provenance_audit` flagged the
instruction. Per §0a item 6.

#### 13.1.5 `ASM-KERNEL-NOT-FOUND`

A `SchedOp::CallKernel` references a kernel not in the
`KernelRegistry`. Severity: Hard.

```rust
ValidationDetail::AsmKernelNotFound {
    kernel_id: KernelSpecId,
    expected_residency: KernelResidency,
    schedop_origin: SchedOpRef,
}
```

Cause: kernel implementation missing in `gbf-kernel`'s registry.
Resolution: implement the kernel (Epic H) or fix the
`SchedulePack` to call an existing kernel.

#### 13.1.6 `ASM-OP-MISSING`

The provenance map does not contain an entry for some
`(mode, slice_id, op_index)` tuple in the SchedulePack.
Severity: Hard.

```rust
ValidationDetail::AsmOpMissing {
    mode: RuntimeMode,
    slice_id: SliceId,
    op_index: usize,
}
```

Cause: lowering rule silently skipped an op. Should be unreachable
in production; reaching this diagnostic means a defect in the
codegen.

#### 13.1.7 `ASM-LEASE-IMBALANCE`

A `PreLayoutOp::BankLease` was emitted without a matching
`PreLayoutOp::BankRelease`. Severity: Hard.

```rust
ValidationDetail::AsmLeaseImbalance {
    section: SectionId,
    acquire_at: usize,
    release_at: Option<usize>,
}
```

Cause: F-A1 `Builder::finish` returned
`BuilderError::UnreleasedBankGuard`. F-B15 propagates with this
code.

#### 13.1.8 `ASM-FARCALL-UNRESOLVED`

A `LegalizationOp::FarCall { bank, target }`'s target symbol
does not resolve to any known section. Severity: Hard.

```rust
ValidationDetail::AsmFarCallUnresolved {
    section: SectionId,
    item_index: usize,
    target_bank: BankIndex,
    target_symbol: SymbolName,
}
```

Cause: codegen emitted a far-call to a symbol that wasn't
emitted (e.g. a kernel that bound to a different residency).

#### 13.1.9 `ASM-CHECKPOINT-UNKNOWN`

A `SchedOp` carries a `SemanticCheckpointId` that is not in
the `SemanticCheckpointSchema` resolved by `SchedulePack
.checkpoint_schema_hash`. Severity: Hard.

```rust
ValidationDetail::AsmCheckpointUnknown {
    schedop_origin: SchedOpRef,
    unknown_id: SemanticCheckpointId,
    schema_hash: Hash256,
}
```

Cause: SchedulePack drift; the upstream observation pass
declared a checkpoint that didn't make it into the schema.

### 13.2 REACH-* — Reachability diagnostics

#### 13.2.1 `REACH-ISR-BANK-DEPENDENCY`

An `IsrReachable` section has residency in a switchable bank.
Severity: Hard.

```rust
ValidationDetail::ReachIsrBankDependency {
    section: SectionId,
    residency: Residency,
    witness_path: Vec<(SectionId, usize)>,
}
```

Cause: a path from an interrupt vector reaches a section
placed in a switchable bank. Resolution: PlacedRom's
residency enforcement (§10.2.5) must place the section in
Bank 0; if `RomWindowPlan` says otherwise, F-B10 is wrong.

#### 13.2.2 `REACH-PRIVILEGED-MBC-WRITE`

A privileged path has an MBC write outside of
`BankLeaseProtected` lease region. Severity: Hard.

```rust
ValidationDetail::ReachPrivilegedMbcWrite {
    section: SectionId,
    item_index: usize,
    witness_path: Vec<(SectionId, usize)>,
}
```

Cause: typically caught by F-A4's audit (§13.1.4); this is a
defense-in-depth check for paths.

#### 13.2.3 `REACH-PRIVILEGE-VIOLATION`

A section's emitted `MachineEffect`s are not allowed by its
declared `PrivilegeClass`. Severity: Hard.

```rust
ValidationDetail::ReachPrivilegeViolation {
    section: SectionId,
    item_index: usize,
    declared: PrivilegeClass,
    forbidden_effect: MachineEffectKind,
}
```

Cause: F-A1 `Builder::validate_effect` would normally have
caught this; reaching here means a builder bypass.

#### 13.2.4 `REACH-PRIVILEGED-SWITCHABLE-DEPENDENCY`

An ISR-reachable or yield-resume-reachable section has a
data load whose target is in a switchable bank.
Severity: Hard.

```rust
ValidationDetail::ReachPrivilegedSwitchableDependency {
    section: SectionId,
    item_index: usize,
    load_target_addr: u16,
    target_bank: BankIndex,
}
```

#### 13.2.5 `REACH-LEASE-REENTRANCY`

A lease region is re-entered without release, or stacked
acquires of overlapping banks occur. Severity: Hard.

```rust
ValidationDetail::ReachLeaseReentrancy {
    outer_lease: LeaseId,
    inner_lease: LeaseId,
    witness_path: Vec<(SectionId, usize)>,
}
```

#### 13.2.6 `REACH-VECTOR-NOT-ROOT`

An interrupt vector's section is not registered as a
reachability root. Severity: Hard.

```rust
ValidationDetail::ReachVectorNotRoot {
    vector: InterruptSource,
    expected_section: SectionId,
}
```

Cause: bead `bd-3s0s` (vectors as roots) not yet closed; F-A1
layout did not pin the vector section.

#### 13.2.7 `REACH-CLASS-DISAGREEMENT`

The computed reachability class disagrees with F-B13's
`ResourceStateValidation` annotation. Severity: Hard. Per §9.6.

```rust
ValidationDetail::ReachClassDisagreement {
    section: SectionId,
    fb13_declared: ResourceClassSet,
    fb15_computed: ReachabilityClassSet,
    witness_path: Vec<(SectionId, usize)>,
}
```

#### 13.2.8 `REACH-CONTINUATION-UNREACHABLE`

A `JP <continuation>` targets a section not reachable from
the corresponding mode's continuation entry. Severity: Hard.

```rust
ValidationDetail::ReachContinuationUnreachable {
    yield_section: SectionId,
    continuation_target: SectionId,
    mode: RuntimeMode,
}
```

#### 13.2.9 `REACH-DEAD-CODE`

A codegen section is unreachable from any root.
Severity: Soft (default) / Hard (when
`CompileObjective.no_dead_code` is set).

```rust
ValidationDetail::ReachDeadCode {
    section: SectionId,
    bytes: u32,
}
```

#### 13.2.10 `REACH-FAULT-PATH-NONRESIDENT-DATA`

A fault-path-reachable section has a data load to a
non-resident address. Severity: Hard.

```rust
ValidationDetail::ReachFaultPathNonResidentData {
    section: SectionId,
    item_index: usize,
    load_target_addr: u16,
    target_bank: BankIndex,
}
```

### 13.3 PLACE-* — Placement diagnostics

#### 13.3.1 `PLACE-EXPERT-TOO-LARGE`

A single expert section group exceeds bank size. Severity: Hard.

```rust
ValidationDetail::PlaceExpertTooLarge {
    expert_id: ExpertId,
    bytes: u32,
    bank_size: u32,
}
```

#### 13.3.2 `PLACE-EXPERT-COMMON-BANK-PRESSURE`

Common-bank packing exceeds capacity under `Budgeted`.
Severity: Hard.

```rust
ValidationDetail::PlaceExpertCommonBankPressure {
    common_bank_capacity: u32,
    common_bank_demand: u32,
    section_count: u16,
}
```

F-B16 may turn this into `RepairProposal::ProfilePromotion(PackedExperts)`.

#### 13.3.3 `PLACE-BANK0-PRESSURE`

Bank 0 demand exceeds capacity. Severity: Hard.

```rust
ValidationDetail::PlaceBank0Pressure {
    bank0_capacity: u32,
    bank0_demand: u32,
    section_summaries: Vec<(SectionId, u32)>,
}
```

#### 13.3.4 `PLACE-ROM-OVERFLOW`

Total ROM byte demand exceeds `TargetProfile.cartridge_profile.
rom_size.bytes()`. Severity: Hard.

```rust
ValidationDetail::PlaceRomOverflow {
    rom_size: u32,
    rom_demand: u32,
}
```

Cross-check with F-B4's static budget: this should have been
caught upstream. Reaching here means F-B4's projection
underestimated.

#### 13.3.5 `PLACE-RELAX-FAILED`

F-A1's relax pass could not converge: an out-of-range branch
cannot be rewritten. Severity: Hard.

```rust
ValidationDetail::PlaceRelaxFailed {
    section: SectionId,
    branch_at: usize,
    target: SymbolName,
    distance_bytes: i32,
}
```

#### 13.3.6 `PLACE-VECTOR-NOT-FIRST-CLASS`

A vector section has no fixed-address pin or has a non-pinned
address. Severity: Hard.

```rust
ValidationDetail::PlaceVectorNotFirstClass {
    vector: InterruptSource,
    section: Option<SectionId>,
    pinned_addr: Option<u16>,
}
```

#### 13.3.7 `PLACE-PROFILE-INFEASIBLE`

The selected `PlacementProfile` cannot place the program.
Severity: Hard.

```rust
ValidationDetail::PlaceProfileInfeasible {
    profile: PlacementProfile,
    reason: ProfileInfeasibilityReason,
}

pub enum ProfileInfeasibilityReason {
    StrictOnePerBankExpertOverflow,
    BudgetedCommonBankOverflow,
    PackedExpertsNoValidPacking,
}
```

F-B16 may turn this into `RepairProposal::ProfilePromotion`
(if its lock-set permits).

#### 13.3.8 `PLACE-FAR-CALL-RESIDENCY`

A `LegalizationOp::FarCall` originates from a section whose
reachability class forbids the call (e.g. an
`IsrReachable` section calling into a switchable bank).
Severity: Hard.

```rust
ValidationDetail::PlaceFarCallResidency {
    caller_section: SectionId,
    caller_class: ReachabilityClassSet,
    target_bank: BankIndex,
    target_symbol: SymbolName,
}
```

#### 13.3.9 `PLACE-COMMON-EXPERT-MIX`

A common-bank kernel and an expert payload were placed in
the same bank in violation of partitioning rules.
Severity: Hard.

#### 13.3.10 `PLACE-ISR-NON-BANK0`

An `IsrReachable` section was placed outside Bank 0.
Severity: Hard.

```rust
ValidationDetail::PlaceIsrNonBank0 {
    section: SectionId,
    assigned_bank: BankIndex,
    reachability_classes: ReachabilityClassSet,
}
```

#### 13.3.11 `PLACE-CONTINUATION-CROSS-BANK`

A `JP <continuation>` was emitted from a switchable section
without going through Bank 0 first. Severity: Hard.

#### 13.3.12 `PLACE-SECTION-CROSSES-BANK`

A section's byte range crosses a bank boundary.
Severity: Hard.

```rust
ValidationDetail::PlaceSectionCrossesBank {
    section: SectionId,
    start_addr: u16,
    end_addr: u16,
    bank: BankIndex,
}
```

#### 13.3.13 `PLACE-EXPERT-RESIDENCY`

An expert payload section was placed in a bank that disagrees
with `RomWindowPlan`. Severity: Hard.

#### 13.3.14 `PLACE-ARENA-OVERFLOW-{WRAM,HRAM,SRAM}`

An arena's total bytes exceed the region capacity.
Severity: Hard. Three variants by region.

#### 13.3.15 `PLACE-CONTINUATION-INVALID`

A continuation target is missing or invalid (cross-check with
§9.3.6 / Rule 6). Severity: Hard.

#### 13.3.16 `PLACE-NONDETERMINISM`

Two regenerations of the same fixture produced different
PlacedRoms. Severity: Hard. Detected by the regeneration
property test.

#### 13.3.17 `PLACE-FUTURE-RESERVATION-COLLISION`

A codegen section was placed into an F-A5-declared
`FutureReservation` byte range. Severity: Hard.

#### 13.3.18 `PLACE-COST-DRIFT`

F-B14's cycle-budget envelope shows >100% utilization for the
placed sections. Severity: Soft in M2; Hard when F-B16 ships.

### 13.4 ENC-* — Encoding diagnostics

#### 13.4.1 `ENC-DRIFT`

The encoded byte count for some section disagrees with
PlacedRom's expected count. Severity: Hard. Catastrophic.

```rust
ValidationDetail::EncDrift {
    section: SectionId,
    expected_bytes: u32,
    actual_bytes: u32,
}
```

#### 13.4.2 `ENC-NONDETERMINISM`

Two encoded ROMs of the same PlacedRom produced different
byte sequences. Severity: Hard. Catastrophic.

#### 13.4.3 `ENC-HEADER-CHECKSUM`

F-A1's ROM builder reported a header-checksum mismatch.
Severity: Hard. Catastrophic.

#### 13.4.4 `ENC-BUILD-HASH-PATCH`

The build-hash patch step (§11.1.4) failed (e.g. the
`build_identity_block` symbol could not be resolved).
Severity: Hard. Catastrophic.

### 13.5 STAGE12-* — Driver-level diagnostics

#### 13.5.1 `STAGE12-INPUT-HASH-MISMATCH`

An input's recorded hash does not match the SchedulePack's
expected upstream-hashes field. Severity: Hard.

```rust
ValidationDetail::Stage12InputHashMismatch {
    field: String,                    // e.g. "schedule_cost_report_hash"
    expected: Hash256,
    actual: Hash256,
}
```

#### 13.5.2 `STAGE12-RESOURCE-STATE-CERT-MISSING`

`certs/resource_state.cert.json` (F-B13's cert) is missing
or not hash-matching. Severity: Hard.

### 13.6 Diagnostic origin variants

F-B15 introduces four `ValidationOrigin` variants:

```rust
pub enum ValidationOrigin {
    // ...existing variants from F-B2/F-B4/F-B11/F-B12/F-B13...
    AsmIRCodegen,           // ASM-* codes
    ReachabilityValidation, // REACH-* codes
    PlacedRomLayout,        // PLACE-* codes
    EncodedRomEmission,     // ENC-* codes
    Stage12Driver,          // STAGE12-* codes
}
```

Per F-B2/F-B4 §5: the variant set is closed; new variants land
via RFC amendment.

## 14. Cross-stage interactions

This section pins the cross-stage seams F-B15 maintains. Each
seam is documented with its direction (consumed / provided), the
specific surface, and the failure mode if the seam breaks.

### 14.1 F-B13 (`SchedulePack` input)

**Direction**: consumed.

**Surface**:

* `SchedulePack` by hash. F-B15 reads `modes`, `epochs`,
  `checkpoint_schema_hash`, `switch_policy`. It does not mutate.
* `certs/resource_state.cert.json` by hash. F-B15 cross-checks
  its computed reachability classes against F-B13's
  annotation-driven classes (§9.6).
* `SchedSlice`/`SchedOp` semantics from F-B13's RFC. Each
  `SchedOp` variant has an F-B15 lowering rule (§8.8.2).

**Failure mode**: if F-B13's cert is missing or hash-mismatched,
F-B15 fails with `STAGE12-RESOURCE-STATE-CERT-MISSING`
(§13.5.2). If F-B13 introduces a new `SchedOp` variant without an
F-B15 lowering update, F-B15 fails with `ASM-LOWERING-MISSING`
(§13.1.1).

**RFC dependency**: F-B13 RFC is forthcoming. F-B15's PR series
should land **after** F-B13's PR series. When F-B13 amends, this
RFC consumes the new surface; if F-B13 changes the
`SchedulePack` shape, F-B15 must amend §6.1, §8.1, §8.8.2,
§14.1.

### 14.2 F-B14 (cost annotations)

**Direction**: consumed.

**Surface**: `ScheduleCostReport` by hash. F-B15 copies
`per_mode` `EstimatedCostDelta` envelopes into `map.json`'s
`cycle_budget` field (§10.8.4). F-B15 does **not** re-derive
costs.

**Failure mode**: if the cost report's per-mode utilization
exceeds 100%, F-B15 emits `PLACE-COST-DRIFT` (§13.3.18). In M2
this is Soft (since F-B16 isn't yet driving retries); in M3+
it becomes Hard.

**RFC dependency**: F-B14 RFC is forthcoming (Chunk 8 in the
sequencing table). F-B15 may consume the cost-report shape by
hash even without F-B14's RFC being final, provided the cost
report's serialization is pinned by `gbf-codegen::cost`'s
existing module stub.

### 14.3 F-B16 (refinement loop — downstream)

**Direction**: F-B16 consumes F-B15's outputs and may request
F-B15 retries.

**Surface**: F-B16 reads F-B15's diagnostics and chooses one
of several `RepairProposal` actions:

* `RepairProposal::ProfilePromotion` — promote
  `Budgeted → PackedExperts` on `PLACE-PROFILE-INFEASIBLE` or
  `PLACE-EXPERT-COMMON-BANK-PRESSURE`.
* `RepairProposal::ReduceTraceDensity` — reduce trace probes on
  `PLACE-COST-DRIFT` or `REACH-FAULT-PATH-NONRESIDENT-DATA`
  (when caused by trace tags inflating fault path).
* `RepairProposal::PromoteOverlay` — promote a kernel to
  `KernelResidency::WramOverlay` on
  `PLACE-EXPERT-COMMON-BANK-PRESSURE` (mediated through F-B11).
* `RepairProposal::AnnotationCorrection` — file a follow-up
  bead correcting F-B13's `ResourceVector` annotation on
  `REACH-CLASS-DISAGREEMENT`.

F-B15 itself does not synthesize repairs; it emits diagnostics
classifying the failure.

**Failure mode**: F-B16 is `BLOCKED on oracle question` (per
F-B11/F-B12 RFC). F-B15 ships without F-B16; the diagnostics
F-B15 emits sit in `repair_report.json` for future F-B16
consumption. This is the current M2 plan.

**RFC dependency**: F-B16 RFC is forthcoming. F-B15 must keep
`PlacementProfile` pluggable (§5.4) so F-B16 can vary it without
amending F-B15.

### 14.4 F-A1 (`gbf-asm` — typed authoring layer)

**Direction**: consumed (heavily).

**Surface**: every type, builder, encoder, layout pass, relax
pass, ROM builder, listing emitter, symbol writer in F-A1's
shipped surface. F-B15 consumes them through their public API.

**Failure mode**: if F-A1 changes any public type, F-B15 must
amend the consuming sections. The version tracking is via
`LAYOUT_ALGORITHM_VERSION` (F-A1's constant) — bumping
invalidates F-B15's K14 cache key (§12.4).

**RFC dependency**: F-A1 has shipped (per F-A5 RFC §0.0.5;
commits `ec10b45`, `53d1d82`, `7a5c687`). F-B15 consumes the
shipped surface. New `Instr`/`SectionRole`/`MachineEffect`
/`PrivilegeClass`/`PreLayoutOp`/`LegalizationOp` variants land
in F-A1 first; F-B15 then consumes them.

### 14.5 F-A2 (`gbf-hw` — target profile, region map, MBC5)

**Direction**: consumed.

**Surface**: `TargetProfile`, `gbf_hw::cartridge_header::*`,
`gbf_hw::memory::*` (region map + predicates),
`gbf_hw::mbc5::*` (MBC5 register addresses + RAM-enable token),
`gbf_hw::interrupts::*` (vector addresses, IE/IF, timer
registers), `gbf_hw::lcd::PpuMode`, `gbf_hw::timing::*`.

**Failure mode**: F-A2 has shipped. Future expansions (e.g. CGB
support) require an F-A2 amendment; F-B15 may then conditionally
consume the new surface (e.g. `is_isr_resident_legal_cgb`).

**RFC dependency**: F-A2 RFC is shipped (commit `a69c2e2`).

### 14.6 F-A3 (`gbf-abi` — live execution contract)

**Direction**: consumed (heavily, for layouts) + provided
(BuildIdentityBlock emission).

**Surface**:
* Consumed: `AbiVersion`, `CompatibilityEnvelope`,
  `BuildIdentityBlock` layout, `InferenceState` prefix layout,
  `LivenessCounters` layout, `HarnessCommandBlock`/
  `HarnessResultBlock` layouts, `FaultCode`/`FaultDomain`
  partition, `TraceEvent` layout, `SemanticCheckpointId`/
  `CompactCheckpointId`, `SemanticCheckpointSchema`.
* Provided: F-B15 emits the `BuildIdentityBlock` at the F-A5-
  defined cartridge offset (§11.1.4).

**Failure mode**: F-A3's compile-time layout assertions
(`static_assertions::const_assert_eq!`) catch any layout
drift inside `gbf-abi`. F-B15's emission asserts the field
offsets against F-A3's `offset_of_*` constants.

**RFC dependency**: F-A3 RFC is shipped (commit `6ad156c`).

### 14.7 F-A4 (`BankLease`/`BankGuard` ABI)

**Direction**: consumed.

**Surface**: `lease_rom_switchable`, `lease_sram`,
`release_bank`, `BankingPreLayoutLowering`, HRAM banking-shadow
constants, `InterruptSafetyTable` declaration substrate,
`mbc_write_provenance_audit`. Per §1.7.

**Failure mode**: if `mbc_write_provenance_audit` flags any
codegen-emitted instruction, F-B15 emits
`ASM-MBC-PROVENANCE-VIOLATION` (§13.1.4). Per §0a item 6.

**RFC dependency**: F-A4 has shipped (commit `6feae98`). F-B15
consumes the shipped API. `KeepCurrentProof` and
`LeaseLifetime::ResumeWindow` / `LeaseLifetime::Token` are
F-A4-deferred surfaces; F-B15 does not need them in M2 closure
scope (§1.7).

### 14.8 F-A5 (Bank0 nucleus + interrupt vectors)

**Direction**: consumed (sections by hash).

**Surface**: F-A5 nucleus sections (boot, interrupts,
scheduler, joypad, text, keyboard, video_commit, panic, ISR
stubs, vector stubs). Plus `runtime_nucleus_hash` and the
`InterruptSafetyTable` declarations.

**Failure mode**: if F-A5 changes the nucleus and the hash
changes, K12 invalidates and F-B15 re-runs codegen.

**RFC dependency**: F-A5 RFC is in flight. F-B15's bead `bd-18d`
depends on F-A5 closure (the Bank0 nucleus must exist before
backend codegen has a runtime to compose with). The bead
`bd-3s0s` (vectors as reachability roots) is the explicit
gate.

### 14.9 F-A7 (`gbf-emu` — consumes encoded ROM)

**Direction**: provided.

**Surface**: F-A7 consumes the `.gb` file plus `map.json` for
`build_identity_block_addr` (to read the
`BuildIdentityBlock`). F-A7's deterministic execution uses
`map.json`'s arena map for breakpoint resolution.

**RFC dependency**: F-A7 RFC is drafted; gbf-emu still
stubbed. F-B15 ships independent of F-A7; the cross-check tests
(`gbf-test`'s integration matrix) gate on F-A7's availability.

### 14.10 F-A8 (`gbf-debug` — consumes `.sym` + `.lst` + `map.json`)

**Direction**: provided.

**Surface**: F-A8's session-file format embeds the entire
`SymbolTable` parsed from `.sym`. The agent CLI's `gb.symbol(name)`
/ `gb.symbol_at(addr)` resolves against the embedded table.
The `.lst` provides the listing context for breakpoints. The
`map.json` provides the arena/region map.

**RFC dependency**: F-A8 RFC is shipped or in flight.

### 14.11 F-F1 (`gbf-report` — build-report aggregator)

**Direction**: provided.

**Surface**: F-F1 reads F-B15's four reports
(`placed_rom_plan.json`, `map.json`, `reachability_report.json`,
`certs/reachability.cert.json`) by hash and aggregates them
into `build_manifest.json`, `provenance.json`,
`compiler_feedback.json`.

**RFC dependency**: F-F1 RFC is forthcoming. F-B15 ships
independent; F-F1's aggregator wires F-B15 outputs as one of
its inputs.

### 14.12 F-F2 (Certificates + cross-validation)

**Direction**: provided.

**Surface**: F-F2 + `gbf-verify` independently re-walks the
reachability graph to validate `certs/reachability.cert.json`.
Two-implementation cross-check is the load-bearing safety
property.

**RFC dependency**: F-F2 RFC is forthcoming (bead `bd-txth`,
which `bd-18d` blocks per §-1).

### 14.13 F-C2 / F-C3 (Oracles)

**Direction**: provided (downstream comparison target).

**Surface**: F-C2 (`ArtifactOracle`) compares vs `QuantGraph`
+ `GbInferIR`, not vs the encoded ROM. F-C3
(`ScheduleOracle`) compares the encoded ROM (or its emulator
execution) against `GbSchedIR`'s interpretation. F-B15
provides the byte-level artifact F-C3 will diff against.

**RFC dependency**: F-C3 RFC is forthcoming. F-B15 ships
independent; F-C3's diff is downstream.

### 14.14 F-D1 / F-D2 / F-D3 / F-D5 (Runtime beyond M0)

**Direction**: provided (placements).

**Surface**:

* F-D1 (persistence) consumes `BuildIdentityBlock` for boot
  validation; persistent SRAM page placements come from
  `ArenaPlan` (F-B12) routed through `map.json.persistent_pages`.
* F-D2 (harness) consumes `harness_command_block_addr` /
  `harness_result_block_addr` from `map.json`.
* F-D3 (trace) consumes `trace_ring_addr` from `map.json`.
* F-D5 (FaultPolicy) consumes per-domain fault entries; F-B15
  provides them as reachability roots in `reachability_report.json`.

**RFC dependency**: F-D* RFCs are forthcoming. F-B15 ships
independent; F-D* plug into the `map.json` schema.

## 15. Task DAG

This section enumerates the task beads under `bd-18d` (F-B15).
Each task is a sub-task PR. The four sub-passes get one task
group each, plus a closing integration task.

### 15.1 Task overview

```text
T-B15.0  Skeleton: gbf-codegen::backend module structure +
         pure-core/driver shape + StageCache key types K12-K15.
         (Lays the layered skeleton; no logic yet.)

T-B15.1  AsmIR codegen — slice-to-AsmIR lowering + pseudo-op
         selection (§8.1, §8.2).
T-B15.2  AsmIR codegen — provenance map + self-consistency
         rules (§8.5, §8.6).
T-B15.3  AsmIR codegen — KernelRegistry binding + RuntimeNucleus
         composition (§8.1.5, §8.1.6).
T-B15.4  AsmIR codegen — emit asmir_summary.json (debug-only)
         + K12 wiring + property tests for byte-stability.

T-B15.5  ReachabilityValidation — edge graph construction +
         class lattice + walker algorithm (§9.1, §9.2).
T-B15.6  ReachabilityValidation — seven validation rules
         (§9.3) + decision procedure (§9.4).
T-B15.7  ReachabilityValidation — F-B13 cross-check (§9.6) +
         disagreement reporting.
T-B15.8  ReachabilityValidation — emit reachability_report.json
         + certs/reachability.cert.json + K13 wiring + property
         tests for class-lattice correctness on synthetic graphs.

T-B15.9  PlacedRom — placement profiles (§10.1) + bank
         assignment (§10.2.1) under StrictOnePerBank +
         Budgeted.
T-B15.10 PlacedRom — far-call thunk insertion (§10.2.2) +
         bank-switch coalescing (§10.2.6).
T-B15.11 PlacedRom — branch relaxation invocation (§10.2.3) +
         stable symbol naming (§10.2.4) + deterministic section
         ordering (§10.2.7).
T-B15.12 PlacedRom — common-bank vs expert-bank partitioning
         (§10.3) + residency enforcement (§10.2.5) + ISR
         residency enforcement against reachability (§10.2.5).
T-B15.13 PlacedRom — PackedExperts profile (§10.1.3) + first-
         fit-decreasing packing.
T-B15.14 PlacedRom — global constraints (§10.5) + continuation-
         target validity check (§10.6).
T-B15.15 PlacedRom — emit placed_rom_plan.json + map.json + K14
         wiring + property tests for determinism.

T-B15.16 EncodedRom — driver (§11.4) + .gb assembly via F-A1
         ROM builder + BuildIdentityBlock patch (§11.1.4).
T-B15.17 EncodedRom — .sym writer wiring (§11.2) + .lst writer
         wiring (§11.3) + K15 wiring + byte-stability test.

T-B15.18 Stage12 driver — orchestrate sub-passes (§4.3) +
         cross-cutting cache discipline (§12.6) + closure-gate
         tests (§0a).
T-B15.19 Integration — wire mbc_write_provenance_audit (§0a item
         6) + F-A4 lease audit pass on emitted bytes.
T-B15.20 Reports — emit and self-hash all four reports +
         certs/reachability.cert.json + property tests for
         schema round-trip.

T-B15.21 Final closure — full property test matrix; M2 fixture
         (one shared common-bank kernel + one expert payload
         bank) compiles end-to-end; all six closure conditions
         (§0a) green.
```

### 15.2 Task ordering

The tasks form a strict DAG:

```text
T-B15.0
   |
   v
T-B15.1 -> T-B15.2 -> T-B15.3 -> T-B15.4   (AsmIR group)
                                    |
                                    v
T-B15.5 -> T-B15.6 -> T-B15.7 -> T-B15.8   (Reachability group)
                                    |
                                    v
T-B15.9 -> T-B15.10 -> T-B15.11 -> T-B15.12 -> T-B15.13 -> T-B15.14 -> T-B15.15
                                                                          |
                                                                          v
                                                              T-B15.16 -> T-B15.17
                                                                          |
                                                                          v
                                                              T-B15.18 -> T-B15.19 -> T-B15.20
                                                                                          |
                                                                                          v
                                                                                      T-B15.21
```

Each arrow is a `blocks` dependency. Within a task, the PR
review packet ships per-task; T-B15.0 is the foundation PR; each
subsequent PR is reviewable in isolation against the task's
acceptance criteria.

The total is 22 tasks (T-B15.0 through T-B15.21) — large by
RFC standards but commensurate with the chunk's contract surface.

### 15.3 Closure milestones inside the bead

* **AsmIR closure** (after T-B15.4): the bundle is produced and
  byte-stable; `asmir_summary.json` round-trips. This is an
  internal milestone — bd-18d does not close until §0a's full
  closure gate is green.
* **Reachability closure** (after T-B15.8):
  `reachability_report.json` and `certs/reachability.cert.json`
  round-trip; the seven rules are tested on synthetic graphs
  per §9.3.
* **PlacedRom closure** (after T-B15.15): `placed_rom_plan.json`
  and `map.json` round-trip; placement determinism property
  tests pass under all three profiles.
* **EncodedRom closure** (after T-B15.17): the M2 fixture
  produces a deterministic `.gb`/`.sym`/`.lst` triple.
* **Bead closure** (after T-B15.21): all six §0a conditions
  green plus the F-A4 audit + the F-B13 cross-check + the
  byte-stability regeneration test.

### 15.4 Per-task review persona routing

Per `CLAUDE.md` §"Reviewer Personas": every bead runs P5
(Proof-of-Work Detective) and P6 (RFC Scope Sentinel)
unconditionally. F-B15 task beads should additionally route:

| Task              | Conditional personas                                   |
|-------------------|-------------------------------------------------------|
| T-B15.0           | P1 (Architecture & Boundary Steward), P2 (Code Cleanliness) |
| T-B15.1–T-B15.4   | P1, P2, P8 (Public Contract / Schema Stability)       |
| T-B15.5–T-B15.8   | P1, P3 (AI Researcher / Experimenter), P4 (QA Engineer), P7 (Numerical & Determinism) |
| T-B15.9–T-B15.15  | P1, P4, P7, P9 (Performance & Resource)               |
| T-B15.16–T-B15.17 | P2, P4, P7                                            |
| T-B15.18–T-B15.20 | P1, P8, P10 (Observability & Telemetry)              |
| T-B15.21          | P5, P6 only (closure gate; structural review)        |

Multi-harness assignments per `CLAUDE.md` table (P1: claude +
codex; P4: gemini + codex; P5: gemini + claude; P6: gemini +
claude; etc.). When personas disagree across harnesses, the
disagreement is itself a signal worth surfacing to a human.

## 16. Rejection classes

This section pins the closure-gate rejection classes per sub-
pass. Each class is a typed diagnostic (§13) plus a closure
predicate that must be tested.

### 16.1 AsmIR sub-pass rejections (§13.1)

| Code | Closure test                                      |
|------|--------------------------------------------------|
| ASM-LOWERING-MISSING | A synthetic SchedulePack with a known-unhandled SchedOp variant produces this code. |
| ASM-NUCLEUS-COLLISION | A codegen path attempts to emit `runtime.boot.entry` (reserved); produces this code. |
| ASM-MBC-RAW-WRITE | A test patch bypasses `Builder::validate_effect` to emit a raw MBC write; produces this code (caught at audit). |
| ASM-MBC-PROVENANCE-VIOLATION | A test patch emits a `StoreToMbcRegister` from a non-banking origin; produces this code. |
| ASM-KERNEL-NOT-FOUND | A SchedulePack references an absent kernel id; produces this code. |
| ASM-OP-MISSING | A codegen path silently skips an op (synthetic patch); produces this code. |
| ASM-LEASE-IMBALANCE | A codegen path acquires without releasing; produces this code. |
| ASM-FARCALL-UNRESOLVED | A SchedulePack far-calls into an unbound symbol; produces this code. |
| ASM-CHECKPOINT-UNKNOWN | A SchedulePack carries an unknown CompactCheckpointId; produces this code. |

### 16.2 Reachability sub-pass rejections (§13.2)

| Code | Closure test                                      |
|------|--------------------------------------------------|
| REACH-ISR-BANK-DEPENDENCY | A synthetic edge graph places ISR-reachable code in a switchable bank; produces this code. |
| REACH-PRIVILEGED-MBC-WRITE | A path from a vector reaches an MBC write outside lease scope; produces this code. |
| REACH-PRIVILEGE-VIOLATION | A `Normal` section is patched to emit a privileged effect; produces this code. |
| REACH-PRIVILEGED-SWITCHABLE-DEPENDENCY | An ISR section reads from switchable address; produces this code. |
| REACH-LEASE-REENTRANCY | A synthetic graph nests overlapping leases; produces this code. |
| REACH-VECTOR-NOT-ROOT | A test environment without bd-3s0s applied produces this code (regression). |
| REACH-CLASS-DISAGREEMENT | A synthetic SchedulePack with intentionally wrong `ResourceVector` annotations produces this code, with both classes named. |
| REACH-CONTINUATION-UNREACHABLE | A codegen path emits a JP to a section that no continuation entry reaches; produces this code. |
| REACH-DEAD-CODE | A synthetic codegen emits an unreachable section; produces this code (Soft by default). |
| REACH-FAULT-PATH-NONRESIDENT-DATA | A fault-handler section reads from switchable address; produces this code. |

### 16.3 PlacedRom sub-pass rejections (§13.3)

| Code | Closure test                                      |
|------|--------------------------------------------------|
| PLACE-EXPERT-TOO-LARGE | A synthetic 24 KiB expert under StrictOnePerBank produces this code. |
| PLACE-EXPERT-COMMON-BANK-PRESSURE | A synthetic Budgeted build with overflowing common bank produces this code. |
| PLACE-BANK0-PRESSURE | A synthetic build with too many Bank0Fixed kernels produces this code. |
| PLACE-ROM-OVERFLOW | A synthetic build exceeding TargetProfile.rom_size produces this code. |
| PLACE-RELAX-FAILED | A synthetic out-of-range JR with no rewrite path produces this code. |
| PLACE-VECTOR-NOT-FIRST-CLASS | A test environment without bd-3s0s applied produces this code. |
| PLACE-PROFILE-INFEASIBLE | A synthetic build under each of the three profiles where infeasible produces the code with the right ProfileInfeasibilityReason. |
| PLACE-FAR-CALL-RESIDENCY | An ISR section's codegen attempts a far-call; produces this code. |
| PLACE-COMMON-EXPERT-MIX | A test environment patched to mix experts and common-kernel produces this code. |
| PLACE-ISR-NON-BANK0 | A synthetic build where ReachabilityValidation marks a section IsrReachable but RomWindowPlan placed it elsewhere produces this code. |
| PLACE-CONTINUATION-CROSS-BANK | A synthetic JP from a switchable section to a continuation produces this code. |
| PLACE-SECTION-CROSSES-BANK | A synthetic 17 KiB section produces this code. |
| PLACE-EXPERT-RESIDENCY | A synthetic expert placed in the wrong bank produces this code. |
| PLACE-ARENA-OVERFLOW-WRAM | A synthetic build with overflowing WRAM arenas produces this code. |
| PLACE-ARENA-OVERFLOW-HRAM | Similar for HRAM. |
| PLACE-ARENA-OVERFLOW-SRAM | Similar for SRAM. |
| PLACE-CONTINUATION-INVALID | Defense-in-depth re-check of REACH-CONTINUATION-UNREACHABLE; produces this code. |
| PLACE-NONDETERMINISM | The regeneration property test asserts byte-equality; intentional perturbation (e.g. randomizing internal ordering) produces this code. |
| PLACE-FUTURE-RESERVATION-COLLISION | A synthetic codegen emit into reserved range produces this code. |
| PLACE-COST-DRIFT | A synthetic SchedulePack whose F-B14 envelope shows >100% utilization produces this code (Soft in M2). |

### 16.4 EncodedRom sub-pass rejections (§13.4)

| Code | Closure test                                      |
|------|--------------------------------------------------|
| ENC-DRIFT | A test patch alters the encoder to disagree with PlacedRom; produces this code. |
| ENC-NONDETERMINISM | The regeneration test asserts byte-equality; intentional perturbation produces this code. |
| ENC-HEADER-CHECKSUM | A test patch corrupts the header bytes after F-A1 ROM builder; produces this code. |
| ENC-BUILD-HASH-PATCH | A test environment without `build_identity_block` symbol produces this code. |

### 16.5 Driver-level rejections (§13.5)

| Code | Closure test                                      |
|------|--------------------------------------------------|
| STAGE12-INPUT-HASH-MISMATCH | A test environment with stale ScheduleCostReport produces this code. |
| STAGE12-RESOURCE-STATE-CERT-MISSING | A test environment without F-B13's cert produces this code. |

### 16.6 Per-sub-pass closure predicates (summary)

* **AsmIR**: every variant of every input enum is exercised; every
  diagnostic class in §13.1 has a positive trigger; the
  byte-stability regeneration test passes.
* **Reachability**: every rule in §9.3 has a positive trigger;
  every class in the lattice §9.1 is reachable on some test;
  the F-B13 disagreement test exercises the §9.6 reconciliation.
* **PlacedRom**: every constraint in §10.5 has a positive
  trigger; all three profiles are tested; the determinism
  property holds.
* **EncodedRom**: byte-stability holds; the build-hash patch
  is round-trippable; F-A1's encoder is invoked exactly once
  per `Instr`; `.sym` and `.lst` round-trip.
* **Driver**: `STAGE12-INPUT-HASH-MISMATCH` and
  `STAGE12-RESOURCE-STATE-CERT-MISSING` have positive triggers.

## 17. Proof obligations

This section enumerates the proof obligations per sub-pass. Each
obligation is a property the implementation must satisfy and
that the closure-gate tests must verify.

### 17.1 AsmIR codegen proof obligations

* **PO-ASM-1: Total coverage.** For every
  `(mode, slice_id, op_index)` tuple in the SchedulePack,
  the provenance map contains at least one entry.
  Tested by §8.6.1's self-consistency rule.
* **PO-ASM-2: No raw MBC writes from codegen.** For every
  `Instr` in `codegen_sections`, if the instruction has
  `MachineEffect::StoreToMbcRegister`, then its provenance
  points to `gbf-runtime::banking`. Tested by F-A4's audit.
* **PO-ASM-3: Lease balance per builder.** F-A1's
  `Builder::finish` returns `Ok(Section)` (not
  `UnreleasedBankGuard`) for every codegen section. Tested
  per-section.
* **PO-ASM-4: Symbol uniqueness.** Every emitted SymbolName is
  unique across nucleus + codegen sections. Tested by
  per-section symbol set construction; collisions produce
  `ASM-NUCLEUS-COLLISION`.
* **PO-ASM-5: KernelRegistry coverage.** For every
  `SchedOp::CallKernel`, the kernel is bound. Tested by
  iterating SchedulePack and checking registry membership.
* **PO-ASM-6: Determinism.** Two codegen runs on the same
  inputs produce byte-equal `AsmIRBundle`s. Tested by the
  regeneration property test.

### 17.2 Reachability proof obligations

* **PO-REACH-1: Lattice convergence.** The forward-flow
  walker terminates in O(N · E · 6) steps. Tested by
  asymptotic-bound tests on synthetic graphs.
* **PO-REACH-2: Class soundness.** A node's class set
  contains exactly the classes whose roots have a path to
  the node. Tested by exhaustive enumeration on small
  synthetic graphs and comparison to a brute-force walker.
* **PO-REACH-3: Rule completeness.** Each of the seven
  validation rules in §9.3 has both a positive and a
  negative test (a graph that satisfies it; a graph that
  violates it). Tested per rule.
* **PO-REACH-4: F-B13 reconciliation.** When F-B13's
  annotation says "lease-protected" and F-B15 computes
  "not lease-protected" (or vice versa), F-B15 emits
  `REACH-CLASS-DISAGREEMENT` with both classes named.
  Tested by synthetic disagreement fixtures.
* **PO-REACH-5: Certificate validity.** The witness in
  `certs/reachability.cert.json` is sufficient for an
  independent walker (in `gbf-verify`) to re-validate.
  Tested by a cross-validator stub in `gbf-verify` that
  consumes the witness.
* **PO-REACH-6: Determinism.** Two reachability runs on the
  same `AsmIRBundle` produce byte-equal reports.

### 17.3 PlacedRom proof obligations

* **PO-PLACE-1: Bank-boundary invariant.** Every section's
  byte range fits in one bank window. Tested per section.
* **PO-PLACE-2: Branch-range invariant.** Every relative
  branch's target is within `±127` bytes after relax.
  Tested by F-A1's relax-pass invariant.
* **PO-PLACE-3: Profile correctness.** Under
  `StrictOnePerBank`, no two experts share a bank. Under
  `Budgeted`, common-bank packing respects slack. Under
  `PackedExperts`, packed-expert co-residency is reachable
  -class-compatible. Tested per profile.
* **PO-PLACE-4: Residency enforcement.** Every section's
  bank assignment matches its `RomWindowPlan` residency
  declaration. Tested per section.
* **PO-PLACE-5: ISR rule enforcement against reachability.**
  Every `IsrReachable` section is in Bank 0 / HRAM / Fixed
  WRAM. Tested by computing reachability and checking
  placement.
* **PO-PLACE-6: Far-call thunk uniqueness.** One thunk per
  `(target_bank, target_symbol)` pair. Tested by counting.
* **PO-PLACE-7: Continuation reachability.** Every yield
  target is reachable from the corresponding mode's
  continuation entry. Tested by re-running the reachability
  walker on the legalized graph.
* **PO-PLACE-8: Determinism.** Two PlacedRom runs on the
  same inputs produce byte-equal results. Tested by
  regeneration.
* **PO-PLACE-9: Future-reservation respect.** No codegen
  section overlaps with F-A5-declared FutureReservation
  ranges. Tested per section.

### 17.4 EncodedRom proof obligations

* **PO-ENC-1: Byte equality with PlacedRom.** Encoded byte
  count per section equals PlacedRom's expected count.
  Tested by per-section comparison.
* **PO-ENC-2: Header checksum validity.** F-A1's ROM builder
  produces correct Pan-Docs checksums. Tested by F-A1's
  closure gate.
* **PO-ENC-3: BuildIdentityBlock patch correctness.** After
  patch, the `build_hash` field equals
  `SHA256(rom_with_field_zeroed)`. Tested by recompute-and-
  compare.
* **PO-ENC-4: Sym/lst round-trip.** `.sym` parses back to
  the same `SymbolTable`; `.lst` is internally consistent
  (every line's bytes match the `.gb`). Tested by parse-back.
* **PO-ENC-5: Determinism.** Two EncodedRom runs produce
  byte-equal `.gb`/`.sym`/`.lst`.

### 17.5 Driver proof obligations

* **PO-DRV-1: Sub-pass ordering.** `run_stage12` invokes the
  sub-passes in the strict order AsmIR → Reachability →
  PlacedRom → EncodedRom. Tested by code review (and by the
  typed product chain in §2.10 enforcing the ordering
  structurally).
* **PO-DRV-2: Failure propagation.** A sub-pass failure
  prevents the next sub-pass from running. Tested by
  injecting failures.
* **PO-DRV-3: Cache discipline.** K12/K13/K14/K15 are
  pinned and tested per §12.7.
* **PO-DRV-4: Idempotency.** Running `run_stage12` twice
  on the same inputs produces the same output. Tested by
  regeneration.

## 18. End-to-end theorem

This section states the load-bearing theorem of the chunk: when
Stage 12 passes, the emitted artifacts form a self-consistent
emission with provenance back to F-B13's SchedulePack, all
banking/lease/residency rules enforced, all sections in-range,
all continuation targets valid, and a deterministic byte order.

### 18.1 Theorem statement

**Theorem (F-B15.End2End)**: Let `S = SchedulePack`,
`P = ResolvedCompilePolicy`, `T = TargetProfile`,
`N = RuntimeNucleusBundle`, `C = ScheduleCostReport`, and
`run_stage12(S, P, T, N, C) = Ok(Stage12Output { asmir, reach,
placed, encoded })`. Then:

```text
1. PROVENANCE.
   For every byte in encoded.gb_bytes outside the cartridge
   header and padding regions, there exists a unique chain:

      byte_offset
        -> map_entry  in placed.map_entries
        -> section    in placed.legalized
        -> AsmIR_item in asmir.codegen_sections OR
                        in asmir.nucleus_sections
        -> source     of one of:
             * SchedSlice/SchedOp/EffectId in S
               (for codegen-emitted bytes)
             * RuntimeShellModule in N (for nucleus bytes)
             * CartridgeHeader / BuildIdentityBlock
               (for the F-A1-builder-emitted header bytes)

   The chain is single-valued and recoverable from
   placed_rom_plan.json + map.json + reachability_report.json.

2. BANKING DISCIPLINE.
   Every Instr in encoded.gb_bytes that has MachineEffect::
   StoreToMbcRegister has provenance pointing to
   gbf-runtime::banking. (PO-ASM-2; verified by F-A4's audit.)

3. LEASE DISCIPLINE.
   Every BankLease acquire has a matching release within the
   same epoch trampoline. (PO-ASM-3; verified per-builder.)

4. ISR RESIDENCY.
   Every byte whose ReachabilityClassSet contains IsrReachable
   has residency in {Bank0, HRAM, FixedWram}. (Rule 1, §9.3.1;
   PO-PLACE-5 enforces.)

5. NO PRIVILEGED MBC WRITES.
   No path from a privileged root reaches a StoreToMbcRegister
   instruction without holding a BankLease. (Rule 2, §9.3.2.)

6. PRIVILEGE-CLASS DISCIPLINE.
   Every section's emitted MachineEffects are in its declared
   PrivilegeClass's allowed set. (Rule 3, §9.3.3.)

7. NO PRIVILEGED SWITCHABLE DEPENDENCY.
   No path from an IsrReachable or YieldResumeReachable root
   has a static-address load whose target is in a switchable
   bank. (Rule 4, §9.3.4.)

8. NO REENTRANCY.
   No edge re-enters a lease region without release; no
   stacked acquires of overlapping banks. (Rule 5, §9.3.5.)

9. CONTINUATION VALIDITY.
   Every JP <continuation> target is YieldResumeReachable from
   the corresponding mode's continuation entry. (Rule 6,
   §9.3.6; PO-PLACE-7 enforces.)

10. FAULT-PATH RESIDENCY.
    Every FaultPathReachable section's data loads target
    addresses in {Bank0, HRAM, FixedWram} (or are
    BankLeaseProtected by an explicit recovery action).
    (Rule 7, §9.3.7.)

11. SECTIONS IN-RANGE.
    Every section's byte range fits in one 16 KiB bank window.
    (PO-PLACE-1.)

12. BRANCHES IN-RANGE.
    Every relative branch's target is within ±127 bytes
    post-relax. (PO-PLACE-2.)

13. ARENAS FIT.
    Total bytes for WRAM hot, WRAM overlay, HRAM, and SRAM
    persistent pages do not exceed their region capacities.
    (PO-PLACE-{1,2}; arena-overflow tests.)

14. ROM SIZE FITS.
    encoded.gb_bytes.len() = T.cartridge_profile.rom_size.bytes()
    (with appropriate padding). (PO-ENC-1; PO-PLACE-overflow.)

15. DETERMINISTIC BYTE ORDER.
    For any two invocations of run_stage12 with byte-equal
    inputs (S, P, T, N, C), the outputs are byte-equal:

       run_stage12(S, P, T, N, C) =
       run_stage12(S, P, T, N, C)

    in particular, encoded.gb_bytes, encoded.sym_lines,
    encoded.lst_text, and the four reports are byte-equal.
    (PO-ASM-6, PO-REACH-6, PO-PLACE-8, PO-ENC-5.)

16. SELF-HASH CONSISTENCY.
    For each report:
       report.report_self_hash = SelfHash(report) =
         DomainHash(crate, type, schema_id, schema_version,
                    canonicalize(report with self_hash =
                    ZERO_HASH))
    (Per F-B2/F-B4 §2.4 self-hash convention.)

17. CERTIFICATE VALIDITY.
    certs/reachability.cert.json's witness is sufficient for
    an independent walker in gbf-verify to re-validate the
    classification and reach the same conclusions.
    (PO-REACH-5.)

18. BUILD IDENTITY EMITTED.
    encoded.gb_bytes contains a BuildIdentityBlock at the
    F-A5-declared offset. The four lineage hashes are filled:
       - build_hash = SHA256(gb_bytes with build_hash = 0)
       - artifact_core_hash = P.artifact_core_hash
       - runtime_nucleus_hash = N.runtime_nucleus_hash
       - compile_request_hash = P.compile_request_hash
    (PO-ENC-3; §11.1.4.)

19. F-B13 RECONCILIATION.
    The reachability classification computed by F-B15 either
    matches F-B13's annotations or emits
    REACH-CLASS-DISAGREEMENT diagnostics naming the path.
    No silent reclassification. (PO-REACH-4; §9.6.)

20. SCHEDULECOST DISCIPLINE.
    map.json.cycle_budget = (subset of) C.per_mode without
    re-derivation. (§10.8.4.)

21. STAGECACHE COHERENCE.
    The four StageCache keys K12, K13, K14, K15 are computed
    per §12. Identical inputs produce identical keys; varying
    any covered field invalidates downstream entries.
    (PO-DRV-3.)
```

### 18.2 Proof sketch

The theorem decomposes into 21 properties. Each property is
discharged by:

* The seven `Rule N` properties (4-10) by ReachabilityValidation
  (§9.3).
* The placement properties (11-14) by PlacedRom (§10.5).
* The provenance property (1) by the AsmIRProvenanceMap (§8.5)
  + PlacedRom's per-section provenance (§10.7) + the address
  mapping in `map.json` (§10.8.3).
* The banking properties (2-3) by F-A4's audit + the codegen's
  self-consistency rules (§8.6).
* The determinism property (15) by the regeneration property
  test (§2.8 + §11.6 + §17's PO-*-determinism).
* The hash properties (16, 18) by the F-B2/F-B4 self-hash
  convention and the build-hash patch (§11.1.4).
* The certificate property (17) by the witness-validator
  cross-check in `gbf-verify`.
* The reconciliation properties (19, 20) by §9.6 and §10.8.4.
* The cache property (21) by §12's key construction rules.

The structural sub-pass ordering (§2.7) combined with the typed
product chain (§2.10) ensures that the discharge of each property
runs in the correct sub-pass; no out-of-order discharge.

The closure gate is **all 21 properties green** in a single
build of the M2 fixture. Failure of any property produces a
typed diagnostic and a Failed-outcome report; the build does
not produce an EncodedRom.

### 18.3 What the theorem does NOT prove

The theorem does **not** prove:

* **Functional correctness** — that the inference results are
  numerically what training produced. That is `ArtifactOracle`'s
  job (F-C2) and `ScheduleOracle`'s job (F-C3).
* **Cycle-budget satisfaction in practice** — that the encoded
  ROM hits its frame budget under real workload. F-B14
  estimates; runtime measurement confirms.
* **Liveness in the operational sense** — that the cooperative
  scheduler makes progress under varying input. F-A5 declares
  the liveness contract; runtime monitors verify it. F-B15's
  contribution is structural (slices have safe yield points)
  not behavioral (slices actually progress).
* **Persistence correctness** — that the persistent SRAM record
  protocol survives torn writes. F-D1 owns persistence; F-B15
  reserves the byte ranges and emits the `lease_sram` calls but
  does not implement the commit-group discipline.
* **Hardware faithfulness** — that the ROM runs on real DMG
  hardware. F-A7 (gbf-emu) approximates real hardware; nightly
  trust tests on real hardware are the final check.
* **Schema migration** — that older artifacts compile under
  newer F-B15 versions. F-A6/F-A6b owns migration; F-B15 fails
  fast on schema mismatch (per F-B2/F-B4's Stage 0 rules).
* **Cross-build comparison** — that two builds with different
  policies produce comparable artifacts. The
  `artifact_core_hash` + `runtime_nucleus_hash` are pinned;
  cross-build comparison is the harness's job, not F-B15's.

### 18.4 The theorem and the closure gate

§0a's closure gate is operationally the theorem's discharge
plus a few engineering-testing additions:

* §0a item 1 (AsmIR determinism) ↔ Property 15 (sub-clause).
* §0a item 2 (Reachability rejects + classifies) ↔ Properties
  4–10, 17.
* §0a item 3 (Placement constraints) ↔ Properties 11–14.
* §0a item 4 (Encoder produces correct .gb/.sym/.lst) ↔
  Property 14 + PO-ENC-{1,3,4}.
* §0a item 5 (StageCache keys) ↔ Property 21.
* §0a item 6 (mbc_write_provenance_audit) ↔ Property 2.
* §0a item 7 (RepairPolicy named-only) — engineering
  invariant; not part of the theorem (no behavioral content).
* §0a item 8 (BuildIdentityBlock fields) ↔ Property 18.

The theorem is the source of truth; §0a is the operational
checklist that green-lights the bead.

### 18.5 Counterexamples (what the theorem rejects)

For clarity, here are concrete scenarios the theorem rejects:

* A SchedulePack where a slice's `live_wram` arena slot
  exceeds the WRAM hot arena's capacity → property 13 fails →
  `PLACE-ARENA-OVERFLOW-WRAM`.
* A RomWindowPlan that places an ISR-reachable kernel in a
  switchable bank → property 4 fails → `REACH-ISR-BANK-DEPENDENCY`
  or `PLACE-ISR-NON-BANK0`.
* A codegen path that emits a raw MBC write (e.g. due to a
  `Builder` bypass) → property 2 fails →
  `ASM-MBC-PROVENANCE-VIOLATION` (caught by F-A4's audit).
* A relax pass that fails to converge → property 12 fails →
  `PLACE-RELAX-FAILED`.
* A regeneration that produces different bytes → property 15
  fails → `PLACE-NONDETERMINISM` or `ENC-NONDETERMINISM`.
* A SchedulePack whose `checkpoint_schema_hash` doesn't include
  a checkpoint a slice references → property 1 fails (chain
  unrecoverable) → `ASM-CHECKPOINT-UNKNOWN`.
* A placement where a section crosses the bank boundary →
  property 11 fails → `PLACE-SECTION-CROSSES-BANK`.

In each case, the theorem's failure surfaces as a typed
diagnostic with a renderable detail and a witness path.

## 19. Final concise contract

### 19.1 What F-B15 promises

**On every successful Stage 12 run**:

1. A `.gb` cartridge image deployable to a DMG/MBC5 cartridge
   and bootable on the target hardware.
2. A `.sym` symbol map consumable by `gbf-debug` for agent
   debugging.
3. A `.lst` listing for human-readable inspection of compiled
   output.
4. `placed_rom_plan.json` — the placement decision record.
5. `map.json` — the load-bearing build artifact for downstream
   tooling.
6. `reachability_report.json` — the per-section/per-byte
   reachability classification.
7. `certs/reachability.cert.json` — the machine-checkable
   certificate of the seven reachability rules.
8. A `BuildIdentityBlock` embedded at the F-A5-declared
   cartridge offset with all four lineage hashes filled.
9. Determinism: byte-identical artifacts under byte-identical
   inputs.
10. Provenance: every byte traceable to its origin in
    `SchedulePack` (codegen) / `RuntimeNucleusBundle` (nucleus)
    / `CartridgeHeader` (header bytes).

**On every failed Stage 12 run**:

11. At least one Hard `ASM-*` / `REACH-*` / `PLACE-*` / `ENC-*`
    / `STAGE12-*` diagnostic with a renderable detail.
12. Per-sub-pass reports for sub-passes that completed (e.g.
    `placed_rom_plan.json` for AsmIR + Reachability successes
    + Placement failure).
13. No `.gb` / `.sym` / `.lst` artifacts when EncodedRom did not
    run.
14. A typed failure mode classifying the rejection (so
    F-B16's eventual repair loop has a typed input).

**Always**:

15. F-A4's `mbc_write_provenance_audit` passes against the
    emitted bytes.
16. F-B13's `certs/resource_state.cert.json` is honored as
    input; disagreements with computed reachability are
    surfaced as `REACH-CLASS-DISAGREEMENT`, never silently
    accepted.
17. F-B14's `ScheduleCostReport` is consumed without re-derivation.
18. The seven reachability rules (§9.3) are enforced
    uniformly across all `PlacementProfile`s; profiles select
    placement strategy, not relaxation surface (§2.13).

### 19.2 What F-B15 does NOT promise

* That the encoded ROM runs correctly on the emulator (F-A7's
  job).
* That the encoded ROM produces correct inference results
  (F-C2/F-C3's job).
* That the ROM hits its cycle budget at runtime (F-E5's job;
  F-B14 estimates).
* That persistence is durable across power cycles (F-D1's job).
* That the ROM signs / authenticates anything.
* That the build is reproducible across compilers / Rust
  versions (the byte-stability guarantee is at the algorithm
  level; system-level reproducibility is a build-system
  concern outside this RFC's scope).

### 19.3 Owner inventory

| Surface                                          | Owner                                       |
|-------------------------------------------------|---------------------------------------------|
| `Instr`, `Section`, `Builder`, encoder           | F-A1                                        |
| `TargetProfile`, MBC5, region map, vectors       | F-A2                                        |
| `BuildIdentityBlock`, `InferenceState`, harness, fault, checkpoint | F-A3                |
| `BankLease`/`BankGuard` ABI, MBC-write audit     | F-A4                                        |
| Bank0 nucleus + interrupt vector stubs           | F-A5                                        |
| `gbf-store` / StageCache implementation          | F-A6                                        |
| `gbf-emu` deterministic emulator                 | F-A7                                        |
| `gbf-debug` agent CLI                            | F-A8                                        |
| `ResolvedCompilePolicy`, `RuntimeChromeBudget`   | F-B2 / F-B4                                 |
| `QuantGraph`, `GbInferIR`                        | F-B3 / F-B5                                 |
| `ObservationPlan`, `RangePlan`, `StoragePlan`    | F-B6 / F-B7 / F-B8                          |
| `SramPagePlan`, `RomWindowPlan`                  | F-B9 / F-B10                                |
| `OverlayPlan`, `ArenaPlan`                       | F-B11 / F-B12                               |
| `SchedulePack`, `ResourceStateValidation`        | F-B13                                       |
| `ScheduleCostReport`                             | F-B14                                       |
| **Stage 12 backend (this chunk)**                 | **F-B15** — AsmIR codegen, Reachability,   |
|                                                  | PlacedRom, EncodedRom, four reports + cert |
| Refinement loop, RepairPolicy                    | F-B16                                       |
| StageCache integration sweep                     | F-B17                                       |
| Build-report aggregation                         | F-F1                                        |
| Certificates + cross-validation                  | F-F2 + `gbf-verify`                         |
| ArtifactOracle                                   | F-C2                                        |
| ScheduleOracle                                   | F-C3                                        |
| Persistence runtime                              | F-D1                                        |
| Harness control plane                            | F-D2                                        |
| Trace pipeline                                   | F-D3                                        |
| FaultPolicy + RecoveryAction                     | F-D5                                        |
| Calibration / Bench                              | Epic E (F-E*)                               |
| Kernels                                          | Epic H (F-H*)                               |

### 19.4 Bead identifiers

* **Feature bead**: `bd-18d` (F-B15: Backend, Stage 12).
* **Blocking edges in (this bead is blocked by)**:
  * `bd-9ae` — F-B13 (SchedulePack input).
  * `bd-1sv` — F-A4 (BankLease ABI).
  * `bd-ssm` — F-A1 (gbf-asm).
  * `bd-2bw` — Epic B parent.
* **Blocking edges out (this bead blocks)**:
  * `bd-3s0s` — T-A1x.2 (vectors as reachability roots).
  * `bd-txth` — F-F2 (Certificates).
* **Task beads**: T-B15.0 through T-B15.21 (§15.1) — to be
  minted at PR-1 of the bead.

### 19.5 RFC's relationship to `planv0.md`

Per §-1's authority rules: where this RFC is more precise, this
RFC wins. The `Amends planv0` notes inline (§2.4 default
profile pinning to `Bringup → StrictOnePerBank`; §10.5.7 future-
reservation byte pattern) are local source-of-truth ledger
entries; they do not request immediate edits to `planv0.md`.

### 19.6 Summary in one sentence

**F-B15 is the deterministic, computational, typed back-end
of the Epic B compiler pipeline that turns a frozen
`SchedulePack` into a deployable LR35902 cartridge image, with
whole-program reachability proven (not declared), placement
pluggable across three profiles, and a tiny encoder that
materializes every choice already made by upstream stages.**

### 19.7 Reading the RFC's spec pack appendix

For implementers, the operational reading order after §0–§7
is:

1. §8 — AsmIR codegen contract + provenance map.
2. §9 — Reachability lattice + seven rules.
3. §10 — PlacedRom layout + global constraints.
4. §11 — EncodedRom serialization.
5. §12 — StageCache keys.
6. §13 — Diagnostic codes.
7. §15 — Task DAG.
8. §17 — Proof obligations.
9. §18 — End-to-end theorem.

For reviewers, the priority is:

1. §0a — closure conditions.
2. §2 — load-bearing decisions.
3. §5 — authority claims.
4. §13 — diagnostics (rejection coverage).
5. §16 — rejection-class testing.
6. §18 — end-to-end theorem.

For downstream consumers (F-A8, F-A7, F-D*, F-F*, F-C*):

1. §10.7 — `placed_rom_plan.json` schema.
2. §10.8 — `map.json` schema.
3. §11 — `.gb`, `.sym`, `.lst` shapes.
4. §9.5 — certificate schema.
5. §14 — cross-stage interactions (find your stage's row).

— end of RFC F-B15 —
