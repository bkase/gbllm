Yes. End-to-end Rust remains the right strategic choice, but the mature form of the design is now:

- shared topology, quantization, packing, LUT generation, charset, and sequence-semantics definitions live in common Rust crates;
- trainer, oracle stack, compiler, runtime authoring, and emulator harness all consume those crates;
- the durable handoff between subsystems is a versioned `ModelArtifact`, and compilation is driven by a separate `CompileRequest`; **do not let build target, compile profile, or calibration leak back into semantic identity**;
- the project is treated as a hardware-aware compiler plus cooperative runtime, not merely a code generator.

The system should be framed as **five cooperating products plus three shared contracts**, with **three semantic strata that cut across those products**, **four object-level layers** that keep every runtime entity unambiguously one thing, and closed by a **measured calibration loop**.

The five products (an ownership/build decomposition — **not**, by itself, the semantic decomposition of the system):

1. a model/training stack,
2. a frozen artifact/export boundary (canonical semantic core + deterministic target-data lowerings + auxiliary sidecars) **plus** an explicit build-request boundary (`CompileRequest`) for target/profile/objective/calibration selection,
3. a three-layer oracle stack (`DenotationalOracle`, `ArtifactOracle`, `ScheduleOracle`),
4. a staged compiler,
5. a cooperative Game Boy runtime.

The three shared contracts remain the semantic backbone of the system:

- `gbf-hw` owns the **target contract**: actual machine/cartridge profile, physical constraints, and calibration schema;
- `gbf-artifact` owns the **durable model contract** only: the deployed artifact lineage (`ArtifactCore`, `TargetDataLoweringArtifact`, `ArtifactAux`) and an optional sibling `ReferenceModelBundle` used only for denotational truth and conformance work;
- `gbf-abi` owns the **live execution contract**: continuation state, harness blocks, fault codes, trace events, semantic checkpoint IDs, a compatibility envelope, and a ROM-resident build identity block.

In addition, three faster-moving operational schema families should stay adjacent to, but not inside, the durable artifact contract:

- `gbf-policy` owns `CompileRequest`, `ResolvedCompilePolicy`, `CompileObjective`, `RiskPolicy`, `RepairPolicy`, and `DeployabilityEnvelope`;
- `gbf-workload` owns `WorkloadManifest`, `ObservationPolicy`, `ExecutionMatrix`, `AcceptanceMatrix`, and prompt-suite schemas;
- `gbf-report` owns `CompiledBuild`, `BuildManifest`, `RunManifest`, `FailureCapsule`, report schemas, and certificates.

For M0-M2, these may remain modules rather than separate crates. The semantic boundary is mandatory even when the physical crate split is deferred.

The three semantic strata (orthogonal to the product decomposition; each stratum is a distinct notion of truth, not a distinct product):

1. **Denotation** — the target-independent reference meaning and quality baseline of the **source reference model**, represented by a durable `ReferenceModelBundle`. This is where "did model quality regress?" lives.
2. **Artifact semantics** — the exact canonical semantics of the frozen exported quantized model, *without* packing, tiling, bank, or schedule assumptions. This is where "did export/quantization introduce a regression?" lives.
3. **Operational schedule** — the resumable, bank-aware execution of those artifact semantics on the target. This is where "did the scheduler, lowering, or backend break the artifact semantics?" lives.

The four object-level layers (every runtime entity in the system is unambiguously exactly one of these):

1. **`ArtifactCore`** — immutable semantic identity of the model. Survives compilation to many targets, many profiles, and many calibration bundles without changing its hash.
2. **`TargetDataLoweringArtifact`** — deterministic derived data form for a target family: packings, swizzles/reorders, block encodings, packed LUT blobs, kernel compatibility metadata. Round-trips to `ArtifactCore` under a declared `packer_version`.
3. **`CompileRequest`** — target/profile/objective/calibration selection, required features, and optional build-constraint overrides. Cheap to vary; does not change artifact identity.
4. **`CompiledBuild`** — a versioned build manifest plus schedule packs, reports, certificates, ROM outputs, listings, maps, and stage-cache refs. Freely replaceable without invalidating semantic lineage.

```rust
pub struct CompiledBuild {
    pub manifest: BuildManifest,
    pub outputs: BuildOutputs,
    pub reports: BuildReportSet,
    pub certs: BuildCertificateSet,
}

pub enum StabilityTier {
    StableContract,
    StableDiagnostic,
    EphemeralDebug,
}
```

### Vocabulary

Clean vocabulary makes design reviews and bug triage much easier:

- **artifact** = semantic identity + deterministic derived data (`ArtifactCore` + any `TargetDataLoweringArtifact`s + `ArtifactAux`);
- **compile request** = how to build it today (`CompileRequest`, including calibration id);
- **build product** = what came out of this compile (schedule, reports, ROM, listings, maps, stage cache entries).

Throughout this document I use **"target-data lowering"** specifically for packings and packed LUT blobs (the deterministic derived data form), to prevent readers from smuggling execution schedule into the artifact boundary.

The five-product decomposition, the three-stratum decomposition, and the four-layer object decomposition are **all** load-bearing and they answer different questions. Product decomposition tells you who owns what and how the workspace is laid out. Stratum decomposition tells you which kind of truth a test, report, or regression is about. Object decomposition tells you whether a given runtime entity is semantic identity, deterministic derived data, a build request, or a build product. Do not collapse them. "Semantic" is not a synonym for "exact executable reference"; see the terminology rule in the Engineering rules section.

The approximation relation is **between** the denotational stratum and the artifact-semantic stratum; it is not embodied inside any single oracle. A dedicated `ConformanceEnvelope` (and a first-class `conformance.json` report) records that relation per workload.

Correctness by construction is retained, but sharpened per stratum:

- local structural illegality belongs in Rust types;
- whole-ROM physical constraints belong in analysis/layout passes;
- **denotational correctness** belongs in `DenotationalOracle`;
- **artifact-semantic correctness** belongs in `ArtifactOracle`;
- **operational correctness** belongs in `ScheduleOracle`, emulator, and hardware comparison;
- no attempt is made to force a whole linker and scheduler into const generics.

Replace the weaker assumptions in the earlier drafts with these corrected axioms:

- deployed ternary weights are modeled as `WeightEncoding::Ternary2` by default;
- normalization is an explicit `NormPlan`, not "a single LUT that somehow is full LayerNorm";
- 16-bit accumulation is a target chosen when proven safe, not a universal assumption;
- "one expert per bank" is a placement profile, not a law of the system;
- sequence state is abstracted behind `SequenceSemanticsSpec`, with physical placement and storage chosen later, so both fixed-state recurrent/linear variants and bounded-KV models can use the same compiler/runtime contract and neither is assumed to automatically win;
- yielding is a compiler-visible ABI, not an implementation detail hidden in handwritten assembly;
- ROM residency under the single-switchable-window rule is planned by a dedicated pass (`RomWindowPlan`), not assumed away;
- the canonical model semantics is target-independent; packing, swizzle/reorder, block encoding, and logical LUT materialization are deterministic **data lowerings** per `DataLoweringProfileId`, not part of the model's identity;
- execution tiling, ROM-window residency, arena assignment, yield slicing, and bank placement are **compile products**, not artifact lowerings;
- `GbInferIR` is a value/effect IR with explicit effect edges, not a place-oriented buffer graph; materialization, buffer, and byte-offset decisions happen in dedicated later passes.

End-to-end Rust is therefore not just "same language everywhere". It specifically means:

- the same canonical specs, ids, and blob schemas are shared end-to-end;
- the same lowering/packer code deterministically materializes target-data lowerings from canonical tensors, whether invoked during export or during compile-time cache population;
- the same production packer and LUT generator are reused where exact deployed bytes must agree;
- verification-critical algorithms *also* have an **independent slow reference implementation** in tests and conformance tooling so a shared bug cannot become self-validating;
- the same `LexicalSpec` drives dataset normalization and charset semantics; the `InteractionBundle` (in `ArtifactAux`) carries keyboard layout (through `KeyboardLayoutSpec`) and transcript rendering (through `TranscriptSpec`);
- the same state-layout types are known to training, oracle stack, compiler, and runtime.

For the training side, Burn remains the pragmatic front-end, but not the owner of deployed quantization semantics. Burn owns backend portability, autodiff, optimizers, checkpoints, and training metrics. `gbf-model` owns ternary projection, activation fake-quant, norm approximation, and export visitation. Burn-native PTQ remains in the workflow as a baseline and ablation path for shared modules (embedding/router/head), but ternary QAT remains custom. Pin an exact Burn version and hide backend/quantization API drift behind a thin internal adapter layer in `gbf-train`. ([Docs.rs][1])

Before the architecture itself, I would lock in three corrections now.

* Ternary is not literally "1 bit" unless you are doing some secondary sparse encoding trick. If the deployed weights are truly `{-1, 0, +1}`, the physical planning number should be 2-bit packed ternary (`WeightEncoding::Ternary2`), or an equivalent two-bitplane representation. So your bank math should assume four weights per byte unless you later prove a better packing.
* A single LUT cannot implement full LayerNorm by itself, because LayerNorm is not an elementwise transform. For this machine, replace full LayerNorm with an explicit `NormPlan` — RMSNorm, affine rescale + clamp, or another normalization variant that the compiler can lower cheaply.
* A 16-bit accumulator is not automatically safe. The compiler needs explicit range analysis (a `RangePlan` pass), because `fan_in × max_abs_activation` can exceed signed 16-bit bounds surprisingly fast.

## The machine you are actually compiling for

On DMG/MBC5, the important physical facts are: a fixed 16 KiB ROM bank at `$0000-$3FFF`, a single switchable 16 KiB ROM window at `$4000-$7FFF`, an 8 KiB external RAM window at `$A000-$BFFF` backed by up to 128 KiB of cartridge RAM across 16 banks of 8 KiB, 8 KiB WRAM, 8 KiB VRAM, and 127 bytes of HRAM. Pan Docs also lists the DMG master clock at 4.194304 MHz with the system clock at one quarter of that, and a frame at 70224 dots, about 16.74 ms. That implies roughly 17.5k normal-speed M-cycles per frame. ([gbdev.io][2])

VBlank begins at LY=144, happens about 59.7 times per second on handheld Game Boy hardware, and lasts only about 1.1 ms (4560 dots ≈ 1140 M-cycles). VRAM is accessible during modes 0, 1, and 2; OAM during modes 0 and 1. Pan Docs also warns that disabling the LCD outside VBlank may damage hardware, and that the first frame after re-enabling stays blank. That means the runtime cannot hide from the single-switchable-window constraint, and inference cannot be treated as "run in VBlank" except for tiny pieces. The correct stance is: keep the display on, let Bank0 own UI, do VRAM/OAM commits in controlled UI phases through an explicit `UiCommitPlan`, and make inference cooperative rather than trying to "freeze the screen and compute." ([gbdev.io][3])

That gives you four immediate architectural rules.

First, Bank0 owns the runtime nucleus: interrupts, scheduler, UI, keyboard, panic screen, and far-call trampolines. It is not automatically the warehouse for every shared weight table. **All ISR code and ISR-reachable data live in Bank0, HRAM, or fixed WRAM only; no interrupt handler may depend on the currently selected switchable ROM or SRAM bank.** This is enforced by a whole-program `ReachabilityValidation` pass in the backend, not by declaration alone.

Second, because only one 16 KiB switchable ROM window is visible at a time, expert-local code and expert-local data placement must be planned by the compiler (`RomWindowPlan`), not assumed. Do not put an expert stub in one bank and its tensors in another by default. ([gbdev.io][2])

Third, SRAM is persistent state, not hot working memory. Hot loops should operate on WRAM-resident tiles and write back to SRAM only at explicit commit boundaries, through a versioned persistent-record protocol with explicit record kinds and page states.

Fourth, yielding must be a compiler feature. The generated inference program should be sliced into resumable chunks with bounded cost, an explicit interrupt-latency budget, **and an explicit liveness contract**; it should not be a monolithic call that hopes interrupts will make everything okay.

## Recommended workspace

I would now structure the workspace like this:

```text
gbforge/
  Cargo.toml
  crates/
    gbf-foundation/  # ids, hashes, semver wrappers, BlobRef, tiny shared enums/newtypes
    gbf-store/       # content-addressed object store, stage-cache implementation, archive/directory transport, pinsets, GC/eviction, integrity verification
    gbf-migrate/     # host-side upgrade DAGs, deprecation windows, artifact/report/workload/calibration migrators
    gbf-hw/          # verified memory map, timing model, calibration schema, target/cartridge profiles, MBC5 registers, LCD/interrupt constants
    gbf-artifact/    # durable model lineage only: canonical semantic core, target-data lowerings, aux sidecars, ReferenceModelBundle
    gbf-policy/      # compile requests, objectives, profiles, repair policy, deployability envelope
    gbf-workload/    # prompt suites, workload manifests, observation policies, acceptance/execution matrices
    gbf-report/      # build products, run manifests, failure capsules, report/certificate schemas
    gbf-abi/         # `repr(C)` live-execution ABI: continuation (with liveness), harness blocks, faults, semantic checkpoints, trace events
    gbf-kernel/      # kernel specs, calling conventions, compatibility ids, slow refs, AsmIR builders, autotune knobs
    gbf-verify/      # independent validators, slow reference algorithms, numeric-profile checks, certificate checking
    gbf-model/       # backend-generic topology, deployable numeric semantics (qat: ternary, activation, norm, export), routing, bank-budget estimators
    gbf-data/        # corpus ingestion, normalization, charset, splits, sampling policies
    gbf-train/       # training/eval/export orchestration, phased QAT, teacher/student, shadow compile, selection, preflight against DeployabilityEnvelope + RuntimeChromeBudget
    gbf-oracle/      # DenotationalOracle + ArtifactOracle + ScheduleOracle, plus conformance/error-envelope tooling
    gbf-ir/          # compiler IRs and analyses; depends on gbf-foundation, not vice versa
    gbf-ir-schema/   # optional serializable view types for stage snapshots and report consumers
    gbf-asm/         # typed LR35902 eDSL, pretty-printer, cycle model, layout support, encoder
    gbf-runtime/     # Bank0/common-bank runtime authored as Rust builders over AsmIR
    gbf-codegen/     # lowering, observe/range/storage/window/kernel/arena/schedule, reachability, backend, legalization, reports; uses gbf-store for stage cache
    gbf-emu/         # emulator adapters, breakpoint orchestration, trace normalization, harness mode
    gbf-test/        # orchestration of integration, property, snapshot, differential, liveness, nightly perf tests
    gbf-bench/       # workload manifests, cycle calibration, calibration bundle production, constrained autotune, Pareto reports
    gbf-cli/         # thin top-level command surface; heavy subcommands feature-gated where practical
  configs/
    model/
    compile/
    bench/
    test/
    calibration/
  fixtures/
    prompts/
    workloads/
    tiny_models/
    golden/
  artifacts/
    builds/
    runs/
    traces/
    reports/
    stage_cache/
  tools/
    flash/
    profile/
    pack/
```

Key decouplings:

- `gbf-model` may know about ML backends and training-time helper modules.
- `gbf-codegen` must not.
- `gbf-runtime` must not know about training frameworks at all.
- `gbf-kernel` owns kernel contracts and kernel-family implementations; `gbf-codegen` selects them and `gbf-runtime` links them, but neither redefines kernel signatures.
- trainer, oracle stack, and compiler all meet at `gbf-artifact` (durable offline contract).
- compiler, runtime, harness, emulator adapters, and `ScheduleOracle` all meet at `gbf-abi` (live execution contract).
- `gbf-hw` owns hardware constants, target profiles, and calibration schema so the compiler and runtime stop carrying magic numbers.
- `gbf-test` exists as its own crate so the correctness loop is not scattered across unrelated packages.
- `gbf-bench` exists as its own crate so workload manifests, cycle calibration, calibration-bundle production, and constrained autotuning are first-class instead of ad hoc scripts.

Inside the important crates, I would make the module split more concrete:

- `gbf-hw::{target, memory, timing, calibration, mbc5, lcd, interrupts, joypad}`
- `gbf-artifact::{manifest, ids, core, aux, lexical, interaction, session, model_spec, quant, sequence, tensors, luts, lowerings, conformance, decode, golden, workload, hint_bundle, compile_request, resolved_policy, deployability}`
- `gbf-abi::{continuation, harness, fault, checkpoint, trace, interrupt, liveness, version}`
- `gbf-kernel::{spec, signature, compat, ref_impl, asm_impl, autotune, calibration}`
- `gbf-asm::{isa, builder, section, provenance, cycle_model, listing, layout, relax, encoder, symbols}`
- `gbf-runtime::{boot, interrupts, scheduler, joypad, text, keyboard, video_commit, banking, panic, trace, harness, persistence}`
- `gbf-codegen::{import, validate, lower_quant, lower_infer, observe, range, storage, window, kernel_select, arena, schedule, lower_asm, reachability, place, legalize, report, rom, stage_cache}`

That one set of boundaries will save you a lot of pain later.

## What each crate is responsible for

`gbf-hw` is the single source of truth for hardware constants, target profiles, and calibration schema: verified memory map, timing model, cartridge profile, MBC5 register semantics, LCD and interrupt constants, and joypad registers. The compiler and runtime should both import these instead of carrying magic numbers.

```rust
pub struct TargetProfile {
    pub console: ConsoleModel,
    pub cartridge: CartridgeProfile,
    pub timing: TimingProfile,
    pub capabilities: CapabilitySet,
}
```

`gbf-hw::calibration` defines the layered calibration schema and timing-model hooks. Concrete calibration bundles (platform, kernel, and runtime layers) are produced by `gbf-bench` and consumed by the compiler through `CalibrationSetRef` in `CompileRequest`:

```rust
pub struct PlatformCalibrationBundle {
    pub id: PlatformCalibrationId,
    pub target: TargetProfileId,
    pub measurement_context: MeasurementContext,
    pub bank_switch_cost: CycleDistribution,
    pub sram_page_cost: CycleDistribution,
    pub timer_isr_cost: CycleDistribution,
    pub confidence: CalibrationConfidence,
    pub valid_for: ValidityEnvelope,
    pub cohort: CalibrationCohortId,
}

pub struct KernelCalibrationBundle {
    pub id: KernelCalibrationId,
    pub target: TargetProfileId,
    pub kernel_impl_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub kernel_profiles: Vec<MeasuredKernelProfile>,
    pub confidence: CalibrationConfidence,
}

pub struct RuntimeCalibrationBundle {
    pub id: RuntimeCalibrationId,
    pub target: TargetProfileId,
    pub runtime_nucleus_hash: Hash256,
    pub scheduler_overheads: SchedulerOverheadModel,
    pub overlay_install_cost: CycleDistribution,
    pub trace_overheads: TraceOverheadModel,
    pub confidence: CalibrationConfidence,
}

pub struct CalibrationSetRef {
    pub platform: PlatformCalibrationId,
    pub kernel: Option<KernelCalibrationId>,
    pub runtime: Option<RuntimeCalibrationId>,
}
```

Calibration is layered by invalidation rate: platform-level facts (bank-switch cost, SRAM page cost, timer ISR cost) are stable across compiler versions; kernel profiles depend on kernel implementation and runtime nucleus hashes; scheduler/overlay/trace overheads depend on the runtime nucleus. Splitting calibration this way avoids discarding perfectly good platform measurements when only compiler internals change. Stale or mismatched calibration at any layer can be rejected or down-weighted. Targets and their calibration schema are data, not constants scattered across passes.

`gbf-artifact` is the durable model-lineage contract crate. Keep it narrow, boring, pure, and versioned.
`gbf-store` owns blob resolution, archive/directory transport, pinsets, GC/eviction, integrity verification, and a two-level always-on `StageCache`:
global legality cache + shard-local caches keyed by named component digests.
Reuse stops at the first truly global stage and resumes only behind declared link barriers. `gbf-artifact` keeps only the schema and references. It should define only durable artifact-lineage types such as `ArtifactManifest`, `ArtifactCore`, `ArtifactSemanticPayload`, `TargetDataLoweringArtifact`, `LoweringShardRef`, `LoweringShard`, `ArtifactAux`, `ReferenceModelBundle`, `ReferenceProgram`, `ReferenceLink`, `ReferenceNumericProfile`, `LexicalSpec`, `InteractionBundle`, `TranscriptSpec`, `SessionProfile`, `DecodeMode`, `DecodeCapabilitySet`, `DecodePolicy`, `ModelSpec`, `QuantSpec`, `SequenceSemanticsSpec`, `CanonicalTensor`, `LogicalLutSpec`, `PackedTensor`, `PackedLut`, `PackedTensorLayout`, `KernelCompatSpec`, `GoldenVector`, `ConformanceEnvelope`, `ReferenceObservationCache`, `SemanticCheckpointSchema`, `HintBundle`, `ExportFacts`, `CompilePreferences`, `BuildConstraints`, `DataLoweringProfile`, `DataLoweringProfileId`, `SidecarRef`, `EvidenceScope`, `SemanticStratum`, `DeterminismClass`, and `ReductionOrderPolicy`. It should be mostly plain Rust data types, `serde`-friendly, and `no_std + alloc` capable.

`CorpusManifest` stays with `gbf-data`; `CompileRequest`/`ResolvedCompilePolicy`/`DeployabilityEnvelope` live in `gbf-policy`; `WorkloadManifest` lives in `gbf-workload`; `CompiledBuild`/`RunManifest`/`FailureCapsule` live in `gbf-report`.

`gbf-abi` owns the shared runtime/compiler contract that is not durable model data: `InferenceState` (with liveness fields), `HarnessCommandBlock`, `HarnessResultBlock`, `FaultCode` (including liveness faults), `SemanticCheckpointId`, `CompactCheckpointId`, `InterruptPolicy`, `TraceEvent`, `AbiVersion`, `CompatibilityEnvelope`, `BuildIdentityBlock`, and `FaultSnapshot`. `gbf-artifact` is the durable offline contract; `gbf-abi` is the live execution contract. Having a separate crate reduces drift between compiler, runtime, harness, emulator adapters, and `ScheduleOracle`. It also gives you a natural home for `#[repr(C)]` layouts and compile-time layout assertions.

`gbf-kernel` owns `KernelSpec`, `KernelSpecId`, calling conventions, tile families, compatibility descriptors, slow reference kernels, production AsmIR builders, and autotune dimensions. `TargetDataLoweringArtifact.kernel_compat` references `KernelSpecId`s rather than ad hoc backend-local names. `gbf-bench` calibrates kernels as declared here; `gbf-codegen` only chooses among legal kernel specs.

`gbf-model` defines the neural architecture once, generic over the Burn backend. This is where you put `MoeCharConfig`, embeddings, router, attention/state-mixing block, expert FFN, and the deployable numeric semantics: `qat::{ternary, activation, norm, export}` modules including `TernaryLinearQat`, `ActFakeQuant`, `NormApproxQat`, `Top1RouterQat`, `ExpertBlockQat`, and `ExportVisitor`. This crate also owns hardware-aware budget estimators such as "packed bytes per expert", "persistent state bytes per layer", and "worst-case accumulator magnitude". It stays backend-generic enough that a different ML backend could slot in without changing the artifact boundary.

`gbf-data` owns the hard-sci-fi corpus pipeline and its governance layer. Normalize text once, deterministically. The training lexical contract derives from `LexicalSpec`; the on-device keyboard derives from `InteractionBundle`. Sampling policies (deterministic seeds, train/val/test splits, canary prompts) live here too, not in ad hoc training scripts. `gbf-data` also owns `CorpusManifest`, source provenance, dedup policy, contamination checks against `WorkloadManifest`s, and license/usage metadata so data drift does not masquerade as model or compiler regressions. The hard-sci-fi "vibe" is a corpus bias and prompt-tuning choice, not an excuse for a sloppy data pipeline.

```rust
pub struct CorpusManifest {
    pub id: CorpusId,
    pub sources: Vec<SourceProvenance>,
    pub normalization: NormalizationSpec,
    pub dedup: DedupPolicy,
    pub contamination: ContaminationPolicy,
    pub split_hashes: BTreeMap<SplitName, Hash256>,
}
```

`gbf-train` is operational. It owns training loops, checkpoints, evaluation, expert-utilization reports, export, phased QAT hardening, and shadow compilation. It is also responsible for emitting the `HintBundle` (`ExportFacts` / `CompilePreferences` / `BuildConstraints`) into the exported artifact, freezing a dense or mixed-precision teacher checkpoint before hard ternarization, exporting that checkpoint as the default sibling `ReferenceModelBundle`, and validating that reference program against the live training model on named fixtures before export succeeds, ingesting build-produced `compiler_feedback.json` artifacts or explicitly promoted feedback bundles from prior builds, and — when available — emitting a `ReferenceObservationCache` for conformance analysis. `gbf-train` also exposes a fast `preflight` path that checks a proposed model/config/checkpoint against a `DeployabilityEnvelope` before long training runs or export. It additionally owns a `shadow_compile` path that, on selected checkpoints, exports the current hard projection, compiles it under one or more `CompileRequest`s, runs a small artifact/schedule conformance suite, and emits `training_selection.json` plus compile feedback for checkpoint selection.

`gbf-oracle` should contain **three** executable specifications, one per semantic stratum:

1. `DenotationalOracle`, which evaluates the frozen backend-free `ReferenceProgram` carried by the sibling `ReferenceModelBundle` in a target-independent reference domain (typically deterministic `f32`/`f64`, with exact arithmetic used for tiny fixtures when practical). It defines the reference meaning and quality baseline. Target independence is the key property, not symbolic maximalism; whole-system proof language is deliberately avoided until there is real proof work to do. The denotational oracle's source of truth is the `ReferenceProgram` inside `ReferenceModelBundle`, not live training code and not the deployed artifact; any `ReferenceObservationCache` sidecar is a cache of observations, not the source of meaning.
2. `ArtifactOracle`, which runs the frozen artifact exactly in canonical logical form: same quantization, same logical LUT semantics, same decode semantics, and same sequence-state semantics, **but without** tiling, bank, or concrete layout assumptions. It proves artifact semantics.
3. `ScheduleOracle`, which runs `GbSchedIR` over the same named arenas, continuation ABI, reduction plans, and semantic checkpoint schema that the ROM will use. It proves scheduled execution semantics before assembly and ROM layout.

The approximation relation is **between** `DenotationalOracle` and `ArtifactOracle`; the middle oracle is not itself an approximation heuristic.
Exact equality gates are legal only when the active `DeterminismClass` admits them; otherwise the gate must be envelope-based, monotonicity-based, or distributional. Each export or build may emit a `conformance.json` report recording error envelopes and agreement metrics on named workloads. Splitting the oracle gives you a tight bug-localization ladder: model quality (denotational) → export/quantization (artifact) → scheduling (schedule) → assembly/backend → emulator/hardware.

`gbf-ir` owns compiler data structures only. No encoding, no layout heuristics, no emulator logic.

`gbf-asm` is your typed LR35902 assembly eDSL plus encoder, cycle model, layout support, branch relaxation, and symbol generation. Executable code should originate here, not from ad hoc byte pushes. See the dedicated Assembly eDSL section below.

`gbf-runtime` is the Bank0/common-bank runtime library, but authored as Rust builders that emit `AsmIR` sections. This is the key interpretation of "end-to-end Rust" on Game Boy: the ROM is still emitted by your backend, but all authoring and orchestration stay in Rust. It must not know about training frameworks at all. The `persistence` module owns the versioned SRAM record protocol; the `banking` module owns the `BankLease`/`BankGuard` ABI that is the only legal path to MBC writes; the `video_commit` module owns the `UiCommitPlan` / video commit queue.

`gbf-codegen` is the actual compiler. Among other things, it owns the always-on content-addressed `StageCache` keyed by semantic core hash + data-lowering hash + compile-request hash + calibration hash + pass version + feature flags; `--resume-from <stage>` is a user-facing control layered on top of that cache.

`gbf-emu` owns emulator adapters, breakpoint orchestration, memory-trace normalization, and the harness-mode execution path that tests consume. It should preferably wrap an existing fast backend plus an existing accuracy/debugger backend rather than grow two emulator cores in-tree. SameBoy advertises very high accuracy with a powerful text debugger (conditional breakpoints, watchpoints, disassembly, backtracing); BGB advertises accurate hardware emulation, clock-exact LCD timing, and `.sym` support. That is the surface you want to stand on, not reimplement. ([GitHub][6])

`gbf-test` is the dedicated home for the correctness loop: integration tests, property tests, snapshot/golden tests, differential tests, liveness stress tests, UI smoke tests, nightly perf/trust tests, and best-effort failure reduction / testcase minimization, all driven by versioned `WorkloadManifest`s where practical.

`gbf-verify` owns the **independent slow reference implementations** for
verification-critical algorithms (pack/unpack, logical LUT evaluation,
decode/RNG semantics, persistence decode, selected range-analysis spot checks)
so a shared bug in the production path cannot become self-validating.

`gbf-bench` owns versioned workload manifests, cycle calibration, layered calibration bundle production (`PlatformCalibrationBundle`, `KernelCalibrationBundle`, `RuntimeCalibrationBundle`), and constrained autotuning. It emits measured kernel profiles, cycle-model drift reports, Pareto-frontier reports across the small set of autotune knobs (tile sizes, kernel residency, slice ceilings), and compile-objective satisfaction summaries. Its goal is to keep the hint bundle from becoming pseudo-scientific decoration and to turn the cost model from a guess into a maintained instrument.

`gbf-cli` is the one place humans interact with.

## The shared artifact boundary

The compiler input should be a self-describing **deployed artifact** with an **immutable canonical semantic core**, zero or more **deterministic target-data-lowering sidecars**, and **mutable auxiliary sidecars**. Compilation itself is driven by a separate `CompileRequest`; target selection, compile profile, objective, and calibration are build inputs, **not** part of semantic identity.
Denotational truth, when preserved, travels as a sibling durable `ReferenceModelBundle` linked from lineage metadata; it is consumed by `DenotationalOracle` and conformance tooling, but never by the compiler. The things that define target-independent execution semantics belong in a content-addressed core; packings/tilings/packed LUT blobs are target-specific derived data lowerings; hint bundles, golden vectors, reference observations, conformance envelopes, and compiler feedback belong in auxiliaries that can evolve without rewriting the core hash.

One careful terminology point: "packed bytes" are not automatically evil. A target-independent canonical byte encoding is fine in the core. The problem is not bytes; the problem is target-committed physical layout. So the core uses `CanonicalTensor` (target-independent), while `TargetDataLoweringArtifact` holds `PackedTensor`, `PackedLut`, `PackedTensorLayout`, and `KernelCompatSpec` values keyed by `DataLoweringProfileId`. Execution tiling, ROM-window residency, arena assignment, yield slicing, and bank placement are **compile products**, not data lowerings — they come out of the compiler, not out of the artifact.

```rust
pub struct ReferenceModelBundle {
    pub manifest: ReferenceManifest,
    pub numeric: ReferenceNumericProfile,
    pub lexical: LexicalSpec,
    pub model: ReferenceModelSpec,   // descriptive / human-meaningful
    pub program: ReferenceProgram,   // frozen executable denotational contract
    pub tensors: Vec<ReferenceTensor>,
    pub decode: DecodeSpec,
}

pub struct ReferenceProgram {
    pub opset: ReferenceOpsetId,
    pub graph: ReferenceEvalGraph,
    pub checkpoint_schema_hash: Hash256,
}

pub struct ReferenceNumericProfile {
    pub scalar_format: ReferenceScalarFormat,
    pub reduction_order: Option<ReductionOrder>,
    pub reduction_order_policy: ReductionOrderPolicy,
    pub rng: ReferenceRngProfile,
    pub determinism: DeterminismClass,
}

pub enum ReductionOrderPolicy {
    Advisory,
    Enforced,
}

pub enum DeterminismClass {
    BitExact,
    NumericallyStable,
    SeedStable,
    DistributionStable,
}

pub struct ReferenceLink {
    pub reference_hash: Hash256,
    pub conformance: Option<ConformanceEnvelope>,
}

pub struct ModelArtifact {
    pub core: ArtifactCore,
    pub lowerings: Vec<TargetDataLoweringArtifact>,
    pub aux: ArtifactAux,
    pub reference: Option<ReferenceLink>,
}

pub struct ArtifactCore {
    pub manifest: ArtifactManifest,
    pub lexical: LexicalSpec,
    pub model: ModelSpec,
    pub quant: QuantSpec,
    pub sequence: SequenceSemanticsSpec,
    pub tensors: Vec<CanonicalTensor>,
    pub luts: Vec<LogicalLutSpec>,
    pub decode_caps: DecodeCapabilitySet,
}

pub struct DataLoweringProfile {
    pub id: DataLoweringProfileId,
    pub target_family: TargetFamilyId,
    pub required_capabilities: CapabilitySet,
    pub kernel_abi: KernelAbiProfileId,
}

pub struct TargetDataLoweringArtifact {
    pub lowering_profile: DataLoweringProfileId,
    pub lowering_hash: Hash256,
    pub packer_version: SemVer,
    pub shards: Vec<LoweringShardRef>,
    pub compatible_targets: BTreeSet<TargetProfileId>,
}

pub struct LoweringShardRef {
    pub component: ComponentId,
    pub hash: Hash256,
    pub len: u32,
    pub codec: BlobCodec,
}

pub struct LoweringShard {
    pub component: ComponentId,
    pub packed_tensors: Vec<PackedTensor>,
    pub packed_luts: Vec<PackedLut>,
    pub packing_layouts: Vec<PackedTensorLayout>,
    pub kernel_compat: Vec<KernelCompatSpec>,
}

pub struct SidecarRef {
    pub hash: Hash256,
    pub len: u32,
    pub codec: BlobCodec,
    pub kind: SidecarKind,
}

pub struct ArtifactAux {
    pub golden: Vec<SidecarRef>,
    pub reference_observation_cache: Option<SidecarRef>,
    pub conformance: Option<SidecarRef>,
    pub semantic_checkpoint_schema: Option<SidecarRef>,
    pub hint_bundle: Option<HintBundle>,
    pub interaction: Option<InteractionBundle>,
    pub promoted_feedback: Vec<SidecarRef>,
}

pub struct SemanticCheckpointSchema {
    pub schema_hash: Hash256,
    pub checkpoints: Vec<SemanticCheckpointDef>,
}

pub struct LexicalSpec {
    pub charset: Charset,
    pub normalization: NormalizationSpec,
    pub control_tokens: ControlTokenSpec,
}

pub struct InteractionBundle {
    pub keyboard: KeyboardLayoutSpec,
    pub transcript: TranscriptSpec,
    pub default_session: Option<SessionProfile>,
}

pub struct TranscriptSpec {
    pub render_policy: RenderPolicy,
    pub fallback_glyph: GlyphId,
    pub show_control_tokens: bool,
    pub wrap_policy: WrapPolicy,
    pub newline_policy: NewlinePolicy,
}

pub enum DecodeMode {
    Argmax,
    TopKTemperature,
}

pub struct DecodeCapabilitySet {
    pub supported: BTreeSet<DecodeMode>,
}

pub struct SessionProfile {
    pub decode: DecodePolicy,
    pub decode_transforms: Option<DecodeTransformSet>,
    pub transcript_policy: Option<TranscriptPolicyId>,
}

pub struct DecodeTransformSet {
    pub repetition_penalty_q8_8: Option<u16>,
    pub stop_sequences: Vec<TokenSequenceId>,
    pub disallow_tokens: BTreeSet<TokenId>,
}

pub enum RngSpec {
    XorShift16,
}

pub struct HintBundle {
    pub facts: Option<ExportFacts>,
    pub preferences: Option<CompilePreferences>,
    pub constraints: Option<BuildConstraints>,
}

pub struct ExportFacts {
    pub scope: EvidenceScope,
    pub workload_scope: WorkloadScope,
    pub sample_count: u32,
    pub expert_usage: Vec<ExpertUsageDigest>,
    pub expert_coactivation: Vec<CoactivationDigest>,
    pub temporal_switch: Vec<TemporalSwitchDigest>,
    pub activation_ranges: Vec<RangeDigest>,
    pub clip_saturation: Vec<ClipSaturationDigest>,
    pub expert_payloads: Vec<ExpertPayloadDigest>,
}

pub struct EvidenceScope {
    pub artifact_core_hash: Hash256,
    pub workload_hash: Option<Hash256>,
    pub target: Option<TargetProfileId>,
    pub lowering_profile: Option<DataLoweringProfileId>,
    pub source_build: Option<Hash256>,
}

pub struct CompilePreferences {
    pub placement_profile: PlacementProfile,
    pub preferred_reductions: Vec<ReductionHint>,
    pub preferred_slice_shape: SliceHint,
    pub router_profile: RouterProfileHint,
    pub expert_slot_affinity: Vec<ExpertSlotAffinity>,
}

pub struct TemporalSwitchDigest {
    pub layer: LayerId,
    pub same_expert_rate_q8_8: u16,
    pub transition_mass: Vec<ExpertTransitionDigest>,
}

pub struct BuildConstraints {
    pub required_features: Vec<CompilerFeature>,
    pub max_bank_switches_per_token: Option<u16>,
    pub max_cycles_per_token: Option<u32>,
    pub max_rom_bytes: Option<u32>,
}

pub struct HintConsumptionReport {
    pub facts_used: Vec<FactUse>,
    pub preferences_honored: Vec<PreferenceUse>,
    pub preferences_ignored: Vec<IgnoredPreference>,
    pub constraints_enforced: Vec<ConstraintEnforcement>,
}

pub struct CompilerFeedback {
    pub scope: EvidenceScope,
    pub fit_margins: Vec<FitMargin>,
    pub bank_switch_hotspots: Vec<SwitchHotspot>,
    pub range_hotspots: Vec<RangeHotspot>,
    pub slice_hotspots: Vec<SliceHotspot>,
    pub cycle_model_drift: Vec<CycleModelDrift>,
    pub autotune_winner: Option<AutotuneWinner>,
}

pub enum SequenceSemanticsSpec {
    LinearState(LinearStateSemantics),
    BoundedKv(BoundedKvSemantics),
}

pub enum PlacementProfile {
    StrictOnePerBank,
    Budgeted,
    PackedExperts,
}

pub enum WeightEncoding {
    Ternary2,
    SparseTernaryBitplanes,
    Binary1,
}

pub struct TernaryWeightPlan {
    pub encoding: WeightEncoding,
    pub scale_granularity: ScaleGranularity,
    pub scale_format: ScaleFormat,
    pub threshold: ThresholdPlan,
}

pub enum ScaleGranularity {
    PerTensor,
    PerOutputRow,
    PerGroup(u16),
}

pub enum ScaleFormat {
    Q8_8,
    Q4_4,
    Pow2,
}

pub enum ThresholdPlan {
    FixedQ8_8,
    AnnealedGlobalThenPerOutputRow,
    LearnedPerGroup(u16),
}

pub enum NormPlan {
    None,
    AffineClipLut,
    TileRmsThenAffineClip,
}
```

### The compile-request boundary

Compilation is driven by a `CompileRequest`, not by smuggling target/profile/calibration into the artifact manifest:

```rust
pub struct CompileRequest {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub calibration: Option<CalibrationSetRef>,
    pub required_features: BTreeSet<CompilerFeature>,
    pub constraint_overrides: Option<BuildConstraints>,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
}

pub struct ResolvedCompilePolicy {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub objective: CompileObjective,
    pub effective_constraints: EffectiveConstraints,
    pub observability: ObservabilityMode,
    pub trace_budget: TraceBudget,
    pub requested_runtime_modes: BTreeSet<RuntimeMode>,
    pub repair: RepairPolicy,
    pub provenance: PolicyProvenance,
}

pub struct PolicyProvenance {
    pub target_defaults: Hash256,
    pub profile_defaults: Hash256,
    pub hint_bundle_hash: Option<Hash256>,
    pub compile_request_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
}

pub enum RuntimeMode {
    Interactive,
    Steady,
    Trace,
    Safe,
}

pub struct CompileObjective {
    pub service: Option<ServiceLevelObjective>,
    pub max_cycles_per_token: Option<u32>,
    pub max_bank_switches_per_token: Option<u16>,
    pub max_sram_page_switches_per_token: Option<u16>,
    pub min_ui_headroom_pct: u8,
    pub max_rom_bytes: Option<u32>,
    pub risk: RiskPolicy,
}

pub struct RiskPolicy {
    pub cycle_quantile: u8,
    pub switch_quantile: u8,
    pub require_confidence_at_least: CalibrationConfidenceClass,
    pub fallback_profile: Option<CompileProfileId>,
    pub fallback_runtime_mode: Option<RuntimeMode>,
}

pub struct ServiceLevelObjective {
    pub max_first_token_cycles_p95: Option<u32>,
    pub max_checkpoint_gap_cycles_p95: Option<u32>,
    pub max_resume_latency_cycles_p95: Option<u32>,
    pub max_ui_jitter_frames_p99: Option<u8>,
}
```

`CompileRequest` is cheap to vary. A single `ArtifactCore` can be compiled against many `CompileRequest`s with different targets, profiles, objectives, and calibration bundles, and every one of those compilations produces a different `CompiledBuild` without invalidating semantic lineage.

### Semantic identity, hashing, and lineage

Two subtle but important points:

The compiler should consume a frozen artifact and a `CompileRequest`, not live training objects.

`ArtifactOracle` and `ScheduleOracle` consume the deployed artifact lineage.
`DenotationalOracle` consumes a sibling `ReferenceModelBundle` plus (optionally) a `ReferenceObservationCache`.
`ReferenceObservationCache` remains a cache of observations; it is never the source of denotational meaning.

**`ArtifactCore` is target-independent.** It carries canonical logical quantized tensors (`CanonicalTensor`), logical LUT contents (`LogicalLutSpec`), and pure sequence **semantics** (`SequenceSemanticsSpec`) — *not* the concrete byte layout of sequence state. It contains nothing about packing, tiling, bank residency, schedule, target profile, compile profile, or calibration. `ArtifactCore` is content-addressed by the canonical serialization of an `ArtifactSemanticPayload`; the `ArtifactManifest` records that hash but is **not** self-hashed recursively. The same rule applies to lowering hashes.

**`ArtifactManifest` should include:**

- schema version (SemVer),
- semantic lineage id and parent hashes,
- canonical payload hash inputs and provenance,
- required features (`BTreeSet<ArtifactFeature>`),
- checkpoint schema hash, recorded from the exported `SemanticCheckpointSchema`, not synthesized only at compile time,
- provenance schema hash,
- tool versions,
- git revisions,
- random seeds,
- corpus digest,
- corpus manifest hash,
- named component digests (experts, router/head, shared LUT families, common kernels) so invalidation can be localized without changing semantic identity,
- deterministic hashes for canonical tensors and logical LUT tables,
- **no target profile id, no compile profile, no calibration reference** (those live in `CompileRequest`, not in the artifact manifest).

**`TargetDataLoweringArtifact` is deterministic and derived.**
Compilation resolves a `TargetProfileId` to a compatible `DataLoweringProfileId`; timing, calibration, and schedule policy remain target-specific, but packed-data identity does not. Any lowering sidecar is hashed separately (`lowering_hash`) and must round-trip back to the semantic core under the declared `packer_version`; `ArtifactValidation` enforces this round-trip. Only deterministic **data** lowerings round-trip this way. For large artifacts, round-trip validation should be defined both per lowering shard and for the assembled lowering manifest. Execution tiling, ROM/build planning, slice boundaries, and autotune winners do not belong here — they are compile products. The compiler can consume a pre-computed lowering when present or deterministically regenerate it when absent or stale, and may lazy-load large blobs through `BlobRef`s rather than eagerly materializing the whole artifact in memory. Packer drift becomes an early validation error instead of a mysterious runtime mismatch.

**`ArtifactAux` is mutable and can be rewritten between builds without invalidating the core.** `ReferenceObservationCache` is optional; when present, `DenotationalOracle` uses it to amortize reference evaluation, but it is a cache, not a source of meaning. `ConformanceEnvelope` / `conformance.json` records quality/degradation relationships between denotation and artifact semantics. The approximation relation lives here, not inside any oracle. `ConformanceEnvelope` defines how much degradation from the reference is acceptable — so the frozen artifact is explicitly *not* the definition of success.

**`LexicalSpec` + `InteractionBundle` replace the old monolithic `InputSpec`.** `LexicalSpec` owns charset, normalization, and control tokens — the model-semantic subset that belongs in `ArtifactCore`. `InteractionBundle` owns keyboard layout, transcript rendering, and default session profile — interaction/policy concerns that live in `ArtifactAux`, not in immutable model identity. This means the same quantized model does not get a new semantic identity hash because you changed line wrapping, fallback glyph handling, or keyboard layout. `KeyboardLayoutSpec` governs on-device input; `TranscriptSpec` governs what actually renders (control-token visibility, fallback glyphs, wrap and newline policies). Otherwise harness output, persistence, and UI rendering will slowly drift. Likewise, active decode policy is explicit through `SessionProfile` / `DecodePolicy` — oracle, compiler, runtime, and harness all need the same decode and RNG semantics at session/run level, while `ArtifactCore` declares only a `DecodeCapabilitySet` of supported decode modes.

**`HintBundle` is load-bearing, not decorative.** The compiler treats facts, preferences, and constraints differently: `ExportFacts` are export-side measured evidence with explicit applicability scope, `CompilePreferences` are cost-function nudges, and `BuildConstraints` are hard admissibility conditions. Target-measured kernel timings belong only in calibration bundles and bench artifacts, never in `ExportFacts`. Every build emits a `hint_consumption.json` listing facts used, preferences honored, preferences ignored, and constraints enforced. The loop is bidirectional, but `CompilerFeedback` is produced first as a build/run artifact (`compiler_feedback.json`) keyed by build and workload. A separate promotion step may attach curated feedback bundles back to artifact lineage when that is intentional. Splitting the old flat `CompileHints` into the three typed buckets is what makes the contract honest: facts decay, preferences get ignored when they conflict with constraints, and constraints are never silently dropped.

On disk, the artifact should be self-describing. I would support:

- a directory form backed by a content-addressed `blobs/sha256/` store, with thin object manifests in `core/`, deterministic `lowerings/<target>/<component>/` shards plus a lowering manifest, and mutable `aux/` / `reports/` sidecars,
- and optionally a single archive form for transport.

```rust
pub struct BlobRef {
    pub hash: Hash256,
    pub len: u32,
    pub codec: BlobCodec,
}

pub enum BlobCodec {
    Raw,
    Zstd,
}

pub struct CanonicalTensor {
    pub layout: CanonicalTensorLayout,
    pub payload: BlobRef,
}

pub struct PackedTensor {
    pub layout: PackedTensorLayout,
    pub payload: BlobRef,
}
```

I would not make opaque "just bincode the whole world" the only archival format. An in-memory Rust struct is perfect. A long-lived storage format should remain inspectable and versioned.

### Workloads

Workloads are first-class, versioned, **strongly typed** artifacts shared by correctness, quality, performance, endurance, persistence, and UI testing. A single generic `acceptance` field would be a junk drawer; workloads are typed by purpose so CI and triage can stay honest.

```rust
pub struct WorkloadManifest {
    pub id: WorkloadId,
    pub class: WorkloadClass,
    pub prompts: Vec<PromptCase>,
    pub seeds: Vec<u64>,
    pub session: SessionProfile,
    pub observation: ObservationPolicy,
    pub execution: ExecutionMatrix,
    pub acceptance: AcceptanceMatrix,
}

pub enum WorkloadClass {
    Conformance,
    Performance,
    Endurance,
    Persistence,
    Ui,
    Quality,
    Interactive,
}

pub struct ObservationPolicy {
    pub checkpoints: CheckpointSelection,
    pub trace_level: TraceLevel,
    pub compare_domain: CompareDomain,
    pub determinism_requirement: DeterminismClass,
}

pub struct ExecutionMatrix {
    pub denotational: bool,
    pub artifact: bool,
    pub schedule: bool,
    pub harness: bool,
    pub hardware: bool,
}

pub struct AcceptanceMatrix {
    pub denotational_vs_artifact: Option<EnvelopeGate>,
    pub artifact_vs_schedule: Option<ExactOrEnvelopeGate>,
    pub schedule_vs_runtime: Option<ExactGate>,
    pub performance: Option<PerformanceGate>,
    pub experience: Option<ExperienceGate>,
    pub recovery: Option<RecoveryGate>,
}

pub struct ExperienceGate {
    pub max_degeneration_rate_pct: u8,
    pub max_abort_rate_pct: u8,
    pub max_first_token_frames_p95: u16,
}

pub struct ConformanceEnvelope {
    pub overall: EnvelopeGate,
    pub per_checkpoint: BTreeMap<SemanticCheckpointId, EnvelopeGate>,
    pub per_metric: BTreeMap<MetricId, EnvelopeGate>,
}

pub enum SemanticStratum {
    Denotational,
    Artifact,
    Operational,
}

pub struct RunManifest {
    pub run_id: RunId,
    pub study_tag: Option<String>,
    pub artifact_core_hash: Hash256,
    pub reference_hash: Option<Hash256>,
    pub compile_request_hash: Option<Hash256>,
    pub build_manifest_hash: Option<Hash256>,
    pub workload: WorkloadId,
    pub execution_backend: ExecutionBackend,
    pub calibration: Option<CalibrationSetRef>,
    pub seed_set_hash: Hash256,
    pub reproducibility_hash: Option<Hash256>,
    pub result_refs: Vec<SidecarRef>,
}

pub struct ReproducibilityManifest {
    pub rustc_version: String,
    pub cargo_lock_hash: Hash256,
    pub host_triple: String,
}

pub struct FailureCapsule {
    pub failing_stratum: SemanticStratum,
    pub workload: WorkloadId,
    pub first_failing_checkpoint: Option<SemanticCheckpointId>,
    pub artifact_core_hash: Hash256,
    pub compile_request_hash: Option<Hash256>,
    pub build_manifest_hash: Option<Hash256>,
    pub refs: Vec<SidecarRef>,
    pub minimized_workload: Option<SidecarRef>,
    pub reduction_log: Option<SidecarRef>,
}
```

`WorkloadClass` tells you the workload's purpose. `ObservationPolicy` tells you which checkpoints are collected and in what numerical domain. `ExecutionMatrix` tells you which execution paths the workload runs against. `AcceptanceMatrix` replaces the old flat acceptance field with explicit per-stratum gates. A conformance workload wants deterministic decode, rich checkpoint collection, and tighter numerical envelopes; a performance workload wants a representative prompt mix, limited probe overhead, and explicit timing backend and calibration reference; an endurance/persistence workload wants long duration and power-loss injection; a UI workload wants dirty-region churn and frame-smoothness constraints. Treating these as the same thing makes CI noisy.

### Deployability envelope (preflight)

A lot of bad model configurations can be rejected or risk-scored before you burn serious time on training:

```rust
pub struct DeployabilityEnvelope {
    pub target_family: TargetFamilyId,
    pub supported_ops: BTreeSet<DeployableOp>,
    pub norm_caps: BTreeSet<NormPlan>,
    pub placement_caps: BTreeSet<PlacementProfile>,
    pub expert_payload_budget: ByteBudget,
    pub state_budget_per_layer: ByteBudget,
    pub routing_caps: RoutingCaps,
    pub reduction_caps: ReductionCaps,
}

pub struct RuntimeChromeBudget {
    pub target: TargetProfileId,
    pub profile: CompileProfileId,
    pub runtime_nucleus_hash: Hash256,
    pub rom_slots: Vec<RomBudgetSlot>,
    pub wram_reserved: u16,
    pub sram_reserved: u32,
}

pub struct RomBudgetSlot {
    pub id: BudgetSlotId,
    pub class: BudgetSlotClass,
    pub usable_bytes: u32,
    pub reserved_slack: u16,
    pub placement_caps: BTreeSet<PlacementProfile>,
}

pub enum BudgetSlotClass {
    Bank0Free,
    CommonBank,
    ExpertBank,
}

pub struct DeployabilityReport {
    pub fits_envelope: bool,
    pub hard_failures: Vec<DeployabilityFailure>,
    pub risk_flags: Vec<DeployabilityRisk>,
    pub projected_fit: ProjectedFitSummary,
}
```

`DeployabilityEnvelope` is not "proof the model fits." It is "this configuration is within the design envelope we know how to compile and deploy." `gbf-train -- preflight` checks a proposed model/config/checkpoint against the target-family `DeployabilityEnvelope` and the build-specific `RuntimeChromeBudget` emitted by the current UI/runtime shell build, and flags unsupported norm modes, impossible state size, over-budget expert payloads, unsupported routing profiles, and activation ranges that are likely to break integer plans — before a long training run or export. This is the best "shift left" improvement in the whole document.

## Model-side recommendations

For the revised plan, I would separate "bring-up profile" from "production target" explicitly — but I would stop claiming to know which will win.

Training stack:

- `gbf-model` owns backend-generic topology plus the deployable numeric semantics: `qat::{ternary, activation, norm, export}`, routing, and bank-budget estimators;
- `gbf-train` owns training/eval/export orchestration, `phases`, `teacher`, `shadow_compile`, `selection`, and preflight against both `DeployabilityEnvelope` and the current `RuntimeChromeBudget`;
- the control plane is Burn-fronted, but the model crate stays backend-generic enough that a different ML backend can slot in without changing the artifact boundary.

Model family:

- **first production candidate**: fixed-state recurrent / linear-attention-like sequence block under `SequenceSemanticsSpec::LinearState`;
- **equally supported alternate production path**: bounded-KV causal model under `SequenceSemanticsSpec::BoundedKv`;
- bring-up still starts under tiny `BoundedKv`, but the architecture does not assume `LinearState` will necessarily win the quality/latency trade;
- deployment default: top-1 routing;
- sparse MoE applies only to the FFN path of selected blocks; the sequence-state update path remains shared and higher precision in the first production model;
- first serious profile uses MoE in alternating or middle blocks rather than every block;
- input embedding and output classifier are tied by default for char/byte models;
- default expert MLP is a two-matrix `d_model -> d_ff -> d_model` block with a clipped nonlinearity; three-matrix GLU variants remain experimental;
- optional stability profile: each MoE block may include a tiny shared dense FFN branch in common banks so the sparse expert acts as a delta rather than the whole feed-forward path;
- router training defaults: low-rank top-1 router, z-loss, expert dropout, and temporal smoothness regularization on contiguous token windows;
- experimental profile: top-2 routing remains allowed only behind an explicit compile profile.

Weight format:

- baseline expert format: `WeightEncoding::Ternary2` plus an explicit `TernaryWeightPlan`; the storage encoding alone is not enough to specify the deployed numeric behavior or byte cost;
- default expert setting: `PerOutputRow` scales in `Q8_8`, with thresholds annealed globally and then refined per output row during QAT;
- the scale tensors themselves are fake-quantized during training and exported as first-class tensors, not silently left in float;
- optional format when zero density is high enough to win in practice: `SparseTernaryBitplanes`;
- router and classifier are allowed to stay at a higher precision than experts if that is the best quality/performance trade.

Normalization:

- the compiler must see the norm strategy as data, not folklore;
- `NormPlan::AffineClipLut` is the simplest deployable path;
- `NormPlan::TileRmsThenAffineClip` is the more faithful path when range analysis and cycle budgets allow it;
- "full LayerNorm is a single LUT" is removed from the plan.

Expert sizing:

- keep the spirit of bank-aware topology;
- stop treating exact 16384-byte equality as a universal law;
- replace it with a soft training objective plus a hard compiler fit check;
- in practice I would budget experts against the smallest `ExpertBank` `RomBudgetSlot::usable_bytes` minus reserved slack, not a hand-written constant;
- for strict one-bank experts, size against the deployed formula `ceil(weight_count / 4) + scale_bytes + metadata_bytes`;
- do not default to GLU experts here: the third projection is too expensive in bank-sized ternary deployments unless later measurements prove otherwise.

I would make the hardware-aware training objective optional, config-gated, and explicitly **M6-scoped** — it is a tuning feature, not a base truth:

```text
loss =
    lm_loss
  + λ_distill  * logit_distillation_loss
  + λ_balance  * expert_load_balance_loss
  + λ_zrouter  * router_z_loss
  + λ_switch   * temporal_switch_penalty
  + λ_range    * activation_range_penalty
  + λ_zero     * ternary_zero_regularizer
  + λ_shape    * structured_shape_penalty
  + λ_overflow * bank_overflow_penalty
```

Default interpretation:

- the router is nudged toward low switch-rate contiguous token trajectories,
- activations are nudged toward safe integer ranges,
- but the compiler still owns final packing, placement, and legality.

`λ_shape` and `λ_overflow` are disabled for fixed-shape `Ternary2` experts, because bank fit is then a geometry/export property, not a differentiable training property. They are enabled only for adaptive-width or sparse-bitplane profiles where exported bytes actually depend on learned structure.

A particularly useful locality term is the temporal switch penalty: `L_switch = (1/LT) Σ_{l,t} (1 - ⟨p_{l,t}, p_{l,t-1}⟩)` where `p_{l,t}` is the soft expert distribution for layer `l`, token `t`. This is a much better proxy for bank-switch pain than a vague "locality loss."

Training proceeds in explicit phases, each with independently controlled QAT hardness:

```rust
pub enum TrainPhaseKind {
    DenseTeacherWarmup,
    RouterWarmup,
    ExpertTernaryQat,
    FullNumericQat,
    HardenAndSelect,
}

pub struct TrainPhaseSpec {
    pub kind: TrainPhaseKind,
    pub start_step: u64,
    pub end_step: u64,
    pub expert_qat: QuantHardness,
    pub activation_qat: QuantHardness,
    pub norm_qat: QuantHardness,
    pub router_mode: RouterTrainMode,
}

pub enum QuantHardness {
    Off,
    Soft,
    Hard,
}

pub enum RouterTrainMode {
    SoftTop1,
    HardTop1,
}

pub struct ShadowCompilePolicy {
    pub every_n_steps: u64,
    pub requests: Vec<CompileRequestRef>,
    pub workloads: Vec<WorkloadId>,
    pub keep_frontier: usize,
}

pub struct CheckpointFrontierPoint {
    pub checkpoint: CheckpointId,
    pub quality: QualitySummary,
    pub conformance: ConformanceSummary,
    pub projected_fit: ProjectedFitSummary,
    pub schedule_cost: Option<EstimatedCostDelta>,
}
```

Phase A (`DenseTeacherWarmup`): no hard ternary, no activation fake quant, router can be soft top-1. Train for quality and stable specialization. Phase B (`RouterWarmup`): freeze the basic geometry, turn on top-1 behavior, load-balance loss, z-loss, expert dropout, and temporal switch regularization on contiguous windows. Phase C (`ExpertTernaryQat`): turn on hard/soft ternary projection for expert up/down projections only, learn or anneal per-row thresholds, fake-quantize the exported scales too, distill from the frozen dense teacher. Phase D (`FullNumericQat`): turn on activation fake quant at expert input/output, residual boundaries, and deployable norm modules; keep range penalties on. Phase E (`HardenAndSelect`): run EMA export, shadow compile, small conformance suite, and frontier selection; export the chosen dense teacher as `ReferenceModelBundle` and the hard ternary student as `ModelArtifact`.

M6-only optional research profile for adaptive bank-aware expert structure:

```rust
pub enum ExpertShapePolicy {
    Fixed,
    StructuredWidthGates { row_group: u16, col_group: u16 },
}
```

Under `StructuredWidthGates`, experts are trained as a supernet and exported after hardening/pruning into bank-fitting widths. This is the regime where `λ_shape` and `λ_overflow` become meaningful training signals.

The dataset side should also become more disciplined:

- `gbf-data` owns normalization and charset construction;
- `LexicalSpec` is the shared lexical contract between training, oracle stack, compiler, and runtime; `InteractionBundle` carries keyboard/transcript policy in `ArtifactAux`;
- sampling policies are deterministic, versioned, and exported alongside the artifact for reproducibility;
- evaluation prompts belong to versioned `WorkloadManifest`s in `fixtures/workloads/`.

## The compiler pipeline

This remains a real staged compiler. The revised pipeline presents a **validation envelope**, a **transform pipeline**, and a **reporting envelope** so the stage story stays honest without losing rigor. Internally, validation and reporting are implemented as first-class passes, but architecturally they bracket the transform pipeline.

Headline pipeline:

**Policy / feasibility envelope:**

0. `ArtifactValidationAndUpgrade`
0.5 `ResolvedCompilePolicy`

**Transformative stages (wrapped by a bounded `FeasibilityRefinementLoop`):**

1. `QuantGraph`
2. `StaticBudgetReport`
3. `GbInferIR` (value/effect IR)
4. `ObservationPlan`
5. `RangePlan`
6. `StoragePlan`
7. `SramPagePlan`
8. `RomWindowPlan`
8.5 `OverlayPlan`
9. `ArenaPlan`
10. `GbSchedIR`
10.5 `ResourceStateValidation`
11. `ScheduleCostAnalysis`
12. Backend (`AsmIR -> ReachabilityValidation -> PlacedRom -> EncodedRom`)

**Reporting envelope:**

13. `BuildReports`

`FeasibilityRefinementLoop` is a bounded monotone repair loop wrapped around
`RangePlan -> StoragePlan -> SramPagePlan -> RomWindowPlan -> ArenaPlan -> GbSchedIR`.
Passes do not call earlier passes recursively; instead they emit
`RepairProposal`s against an explicit `CompileKnobs` surface. `ScheduleCostAnalysis` is the only producer of objective-facing cost envelopes used by the refinement loop, which keeps repair logic centralized and testable.

```rust
pub struct RepairPolicy {
    pub max_refinement_iters: u8,
    pub allow_profile_fallback: bool,
    pub allow_trace_demotion: bool,
    pub allow_overlay_promotion: bool,
    pub allow_recompute_promotion: bool,
}

pub struct RepairProposal {
    pub source: PlanningStage,
    pub reason: RepairReason,
    pub tighten: ConstraintDelta,
    pub estimated_cost: EstimatedCostDelta,
}

pub struct CostEstimate {
    pub p50: u32,
    pub p90: u32,
    pub p99: u32,
    pub evidence: EvidenceClass,
    pub sample_count: u32,
    pub refs: Vec<EvidenceRef>,
}

pub struct EstimatedCostDelta {
    pub cycles: Option<CostEstimate>,
    pub bank_switches: Option<CostEstimate>,
    pub sram_page_switches: Option<CostEstimate>,
    pub bytes: Option<CostEstimate>,
}

pub enum EvidenceClass {
    Measured,
    Transferred,
    Heuristic,
}

pub struct ComponentDigestSet {
    pub components: BTreeMap<ComponentId, Hash256>,
}

pub struct BuildShardManifest {
    pub shard_order: Vec<ShardId>,
    pub shard_products: BTreeMap<ShardId, ShardBuildProductRef>,
    pub final_link_barrier: FinalLinkBarrier,
}
```

Every build emits `policy_resolution.json`, recording the fully resolved
compile policy and the provenance of every hard and soft constraint.

The stage ordering is deliberate. `ObservationPlan` runs immediately after `GbInferIR` so semantic checkpoints and debug probes are derived from canonical model paths before any scheduling happens. `RangePlan` is purely logical and can run next because it does not depend on materialization or residency decisions. `StoragePlan` is the missing bridge from value/effect semantics to spatial scheduling — it decides materialization, persistence, and alias classes without yet assigning bytes. `RomWindowPlan` then uses storage decisions to resolve kernel residency under the single-switchable-window constraint. `ArenaPlan` only then allocates concrete byte ranges to the materialized values `StoragePlan` selected, never to pure expression nodes directly. The backend adds a new sub-pass, `ReachabilityValidation`, that computes (rather than trusts) whole-program reachability classes before final placement.

### Artifact/contract evolution

Versioning is necessary but not sufficient. Every durable offline contract
must also have an explicit upgrade path. Core crates define current schemas
only; host-side migration logic lives in `gbf-migrate` so the compiler,
oracles, and report consumers can assume "current schema in memory."

```rust
pub struct CompatibilityEpochs {
    pub artifact: u16,
    pub abi: u16,
    pub calibration: u16,
    pub reports: u16,
}

pub struct MigrationReport {
    pub input_schema: SemVer,
    pub output_schema: SemVer,
    pub migrators: Vec<MigratorId>,
    pub lossy_fields: Vec<FieldPath>,
    pub warnings: Vec<MigrationWarning>,
}

pub enum MigrationLossClass {
    Lossless,
    LossyButAccepted,
    Rejected,
}
```

### 0. `ArtifactValidationAndUpgrade`

Verify the artifact's schema version, semantic core hash, `ArtifactManifest` invariants, required-feature set, and the self-consistency of canonical tensors, logical LUTs, quant spec, sequence semantics, decode capabilities, lexical spec, interaction bundle, hint bundle, workload manifests referenced by id, and golden vectors before the compiler touches anything else. Any `TargetDataLoweringArtifact` must also be validated against the semantic core (including `packer_version` round-trip) before compilation begins. Validate `CompileRequest` against the resolved `TargetProfile`, required-feature set, and any referenced calibration bundles in the `CalibrationSetRef` (rejecting stale or mismatched calibration at any layer). If the artifact or compile request is broken, fail fast with a clear diagnostic.

### 1. `QuantGraph`

This is the canonical artifact graph:

- frozen canonical tensors only,
- explicit quant formats,
- explicit norm plans,
- explicit sequence-state semantics,
- explicit decode spec,
- explicit provenance back to exported tensor ids.

It is no longer a training graph. All weights are already frozen, all quant schemes are explicit, and every tensor has a canonical target-independent representation. Physical packings, reorders, and bank-chunked lowerings are separate target-data materializations, not part of this graph. This layer should know about tensor shapes, quant ranges, sequence-state semantics, routing tables, expert sections, decode plan, and provenance back to model nodes and exported tensor IDs.

### 2. `StaticBudgetReport`

A dedicated pre-lowering pass that runs before real code generation and answers:

- does each expert fit under the requested placement profile?
- what are projected WRAM / SRAM / HRAM needs?
- what are predicted accumulator maxima?
- what are predicted bank-switch counts per token?
- what is the likely common-bank footprint?

If a budget is busted here, you get a useful error before committing to lowering.

### 3. `GbInferIR` (value/effect IR)

This is the hardware-aware **value/effect IR**. It is still not register allocation, not concrete storage assignment, and not final assembly. It expresses **value dependencies** and **explicit semantic effects**, but it does **not** commit to concrete buffers, tiling, accumulator scratch, or byte offsets. Storage-free, but not effect-free: sequence state mutation and RNG progression are observable semantic effects and they live on explicit effect edges, not hidden in buffer aliasing.

```rust
pub struct GbNode {
    pub op: InferOp,
    pub inputs: Vec<ValueId>,
    pub effects_in: Vec<EffectId>,
    pub outputs: Vec<ValueId>,
    pub effects_out: Vec<EffectId>,
}

pub enum InferOp {
    Embedding { token_src: TokenSrc },
    SequenceRead { slot: StateSlotId },
    SequenceWrite { slot: StateSlotId },
    RouteTop1 { layer: LayerId },
    ExpertMatVec { layer: LayerId, expert: ExpertId },
    CombineResidual,
    Norm { plan: NormPlan },
    Classify,
    DecodeToken { plan: DecodeSpec },
}
```

This IR is typed by **value kind**, **quant format**, and **effect class**. Concrete address space, concrete buffers, tiling, and accumulator scratch belong to later storage/schedule stages, not here. This stage gives downstream passes enough information to derive observations, do range inference, make materialization decisions, plan ROM windows, and schedule. The graph is comparable op-for-op against `ArtifactOracle` output because neither has committed to storage.

### 4. `ObservationPlan`

`ObservationPlan` is new and essential. It consumes the exported `SemanticCheckpointSchema` and answers the questions "which semantic checkpoints are mandatory for this build? in what numerical domain? with what encoding? what additional operational probes and metrics are enabled?" explicitly, instead of letting checkpoint behavior become an emergent property of the scheduler, backend, or harness.

```rust
pub struct ObservationPlan {
    pub semantic: Vec<SemanticObservation>,
    pub probes: Vec<OperationalProbe>,
    pub metrics: Vec<MetricProbe>,
}

pub struct SemanticObservation {
    pub checkpoint: SemanticCheckpointId,
    pub source: ObservationSource,
    pub encoding: ObservationEncoding,
}

pub struct OperationalProbe {
    pub probe_id: TraceProbeId,
    pub source: ProbeSource,
    pub level: ProbeLevel,
    pub budget_class: ProbeBudgetClass,
}

pub struct MetricProbe {
    pub metric: MetricId,
    pub source: MetricSource,
    pub aggregation: MetricAggregation,
}
```

This pass consumes the stable exported semantic-checkpoint schema, chooses required semantic observations for the active build, and derives optional operational/debug probes (`TraceProbeId`s) from the active build profile. It gives you two wins: semantic checkpoint stability even as scheduling changes, and profile-tunable debug instrumentation without changing semantic comparison contracts. It re-emits the selected `semantic_checkpoint_schema.json` and the build-specific `operational_probe_schema.json` as part of the report pack.

### 5. `RangePlan`

`RangePlan` is purely logical. It chooses a `ReductionPlan` per hot reduction without needing any materialization or residency decisions, which is why it runs before `StoragePlan` and `RomWindowPlan`:

```rust
enum ReductionPlan {
    SingleI16,
    ChunkedI16 { chunk_len: u16 },
    RenormLoop { tile_len: u16 },
}
```

This is where the earlier "just use 16-bit accumulators" advice becomes precise and trustworthy. Each reduction gets proven safe either directly at `i16`, in chunked `i16` slabs with intermediate downcasts, or via explicit renormalization loops. The outputs of `RangePlan` feed `StoragePlan` (because a `RenormLoop` reduction may imply different scratch materialization than a `SingleI16` reduction) and `GbSchedIR` (because tile shapes may change).

### 6. `StoragePlan`

`StoragePlan` is the missing bridge between value/effect semantics and spatial scheduling. It decides which values are materialized, which are recomputed, what their lifetime class is (slice, resume window, token, session, or persistent), and which storage class they require, but it still does **not** assign concrete byte offsets.

```rust
pub enum StorageClass {
    WramHot,
    HramHot,
    SramPaged,
    RomConst,
}

pub enum LifetimeClass {
    Slice,
    ResumeWindow,
    Token,
    Session,
    Persistent,
}

pub enum Materialization {
    Recompute,
    Materialize { class: StorageClass, lifetime: LifetimeClass },
    Persist { page: PersistPageId, commit_group: CommitGroupId },
}

pub struct StorageBinding {
    pub value: ValueId,
    pub materialization: Materialization,
    pub alias_class: AliasClassId,
}
```

`Materialization::Recompute` makes recomputation-vs-spill an explicit decision rather than a side effect of buffer lowering. `Materialization::Persist { page, commit_group }` maps persistent semantic state onto the SRAM persistence protocol's `PersistPageId`s without committing to byte ranges here, and groups related pages under a shared `CommitGroupId` for atomic commit. `Materialization::Materialize { class, lifetime }` is the regular case; `LifetimeClass` replaces the earlier `survives_yield: bool` with a richer spectrum (slice, resume window, token, session, persistent) so arena planning and persistence semantics are both cleaner.

This is the clean boundary between "what is the computation" and "where does it live":

- `GbInferIR` is timeless and place-less;
- `ObservationPlan` fixes observation contracts;
- `RangePlan` chooses logical reduction structure;
- `StoragePlan` decides materialization, persistence, and aliasing;
- `RomWindowPlan` uses those decisions to resolve kernel/LUT residency;
- `ArenaPlan` assigns actual byte ranges to the materialized values;
- `GbSchedIR` commits mutation, aliasing, and resumable control flow.

Recomputation-vs-spill becomes an explicit decision, not a side effect of buffer lowering. Schedule equivalence becomes easier to reason about because you can compare value/effect-level semantics directly against bufferized schedule semantics.

### 7. `SramPagePlan`

`SramPagePlan` plans active SRAM working sets, page-switch batching, cold-spill residency, and commit ordering for all `SramPaged` materializations before byte ranges are assigned. This is the SRAM analogue of `RomWindowPlan`: only one 8 KiB external RAM window is visible at `$A000-$BFFF`, so SRAM page working sets and page-switch behavior need a first-class planning stage just as ROM visibility does.

```rust
pub struct SramPagePlan {
    pub active_sets: Vec<SramWorkingSet>,
    pub page_bindings: Vec<SramPageBinding>,
    pub commit_boundaries: Vec<CommitBoundary>,
    pub spill_policy: SpillPolicy,
}
```

This pass makes page families, spill/page rotation, commit batching, and page-switch budgets explicit and optimizable rather than leaving SRAM pressure as a blind spot spread across `StoragePlan`, persistence rules, and scheduling.

### 8. `RomWindowPlan`

`RomWindowPlan` is essential on MBC5. It computes which ROM objects must be simultaneously visible under the single-switchable-window rule (Bank 0 fixed at `$0000-$3FFF`, one switchable 16 KiB window at `$4000-$7FFF`), selects kernel and LUT residency from `StoragePlan`'s materializations, duplicates or stages tiny hot tables when legal, and rejects impossible code/data placements before layout.

```rust
pub enum KernelResidency {
    Bank0Fixed,
    WramOverlay,
    CoResidentSwitchable,
}
```

This pass is what prevents the contradiction that would otherwise live unaddressed in the runtime story: you cannot have "shared micro-kernels in common banks" **and** "expert-local data in expert banks" be the hot-path default simultaneously, because there is only one switchable ROM window at a time. If expert weights occupy the switchable window, the hot expert kernel must execute from Bank 0, from a WRAM overlay, or be co-resident with the expert bank. The same is true for tiny hot LUTs used inside that loop.

`RomWindowPlan` resolves the contradiction per phase: it computes the simultaneously visible ROM set for each hot operation, decides kernel residency, decides LUT residency, and makes the Bank 0 and WRAM overlay budgets honest before any later pass commits to a layout. This also tends to improve performance because it replaces accidental bank thrash with explicit co-residency or staging.

### 8.5 `OverlayPlan`

`OverlayPlan` turns `KernelResidency::WramOverlay` from a residency choice into an explicit install/layout plan. It decides which overlayable objects share a region, when installs may occur, and what WRAM budget must be reserved for overlays before arena assignment begins.

```rust
pub struct OverlayPlan {
    pub regions: Vec<OverlayRegion>,
    pub installs: Vec<OverlayInstall>,
    pub share_classes: Vec<OverlayShareClass>,
}
```

### 9. `ArenaPlan`

`ArenaPlan` assigns named arenas and concrete byte ranges **to the materialized values selected by `StoragePlan`** and reserves the WRAM regions selected by `OverlayPlan`; it does not allocate pure expression nodes directly. The named arenas it manages include:

- ping-pong activations,
- accum scratch,
- route scratch,
- decode scratch,
- continuation record (including liveness fields),
- persistent sequence state pages,
- trace pages,
- harness command/result blocks.

Arenas are named so that later stages — scheduling, placement, trace, runtime, and `ScheduleOracle` — all agree on the same storage geometry.

### 10. `GbSchedIR`

`GbInferIR` says what values and effects must be computed. `ObservationPlan` says what must be observable. `RangePlan` says how reductions are structured. `StoragePlan` says what must be materialized, recomputed, or persisted. `GbSchedIR` says how those materializations are realized as loads, stores, tiles, reductions, and resumable slices. This is where mutation, aliasing, slices, and resumable control flow become explicit.

Each slice is an explicit coroutine boundary and carries an explicit interrupt policy:

```rust
pub enum ResourceLeaseKind {
    RomWindow(RomWindowBinding),
    SramPage(SramPageBinding),
    Overlay(OverlayId),
    InterruptMask(InterruptPolicy),
}

pub struct ResourceLease {
    pub id: LeaseId,
    pub kind: ResourceLeaseKind,
    pub acquired_in: SliceId,
    pub released_in: SliceId,
    pub yield_safe: bool,
}

pub struct ResourceVector {
    pub bank_switches: u16,
    pub sram_page_switches: u16,
    pub trace_bytes: u16,
    pub persist_bytes: u16,
    pub overlay_installs: u8,
}

pub struct SchedSlice {
    pub id: SliceId,
    pub ops: Vec<SchedOp>,
    pub hard_cycles_to_safe_point: CycleBudget,
    pub soft_target_cycles: CycleBudget,
    pub max_interrupt_latency: CycleBudget,
    pub resources: ResourceVector,
    pub live_wram: Vec<ArenaSlot>,
    pub live_sram: Vec<ArenaSlot>,
    pub yield_kind: YieldKind,
    pub yield_check: YieldCheckClass,
    pub entry_residency: Residency,
    pub interrupt_policy: InterruptPolicy,
    pub required_leases: Vec<LeaseId>,
    pub exit_kind: ExitKind,
}

pub enum YieldKind {
    Micro,
    Frame,
    NeedInput,
    TokenReady,
    Finished,
    Fault,
}

pub enum InterruptPolicy {
    Enabled,
    ShortCriticalSection,
    Disabled,
}

pub struct ResidencyEpoch {
    pub id: EpochId,
    pub rom_window: RomWindowBinding,
    pub overlay: Option<OverlayId>,
    pub residency: Residency,
    pub slices: Vec<SliceId>,
}

pub struct SchedulePack {
    pub modes: BTreeMap<RuntimeMode, GbSchedIR>,
    pub epochs: BTreeMap<RuntimeMode, Vec<ResidencyEpoch>>,
    pub checkpoint_schema_hash: Hash256,
    pub switch_policy: ModeSwitchPolicy,
}

pub struct ModeSwitchPolicy {
    pub legal_switch_points: Vec<SemanticCheckpointId>,
    pub legal_epoch_boundaries: Vec<EpochId>,
    pub ui_pressure_thresholds: Vec<UiPressureThreshold>,
    pub safe_mode_triggers: Vec<SafeModeTrigger>,
    pub drift_triggers: Vec<DriftTrigger>,
}

pub struct RuntimeDriftMonitor {
    pub expected: DriftEnvelope,
    pub observed: DriftEnvelope,
    pub consecutive_violations: u8,
}

pub struct DriftEnvelope {
    pub slice_cycles_p95: Option<u32>,
    pub ui_commit_cycles_p95: Option<u32>,
    pub trace_drop_rate_pct: Option<u8>,
    pub persist_overrun_rate_pct: Option<u8>,
}

pub enum DriftAction {
    ShrinkSlices,
    DropTrace,
    DemoteMode(RuntimeMode),
}

pub struct DriftTrigger {
    pub metric: DriftMetric,
    pub threshold: u32,
    pub action: DriftAction,
}
```

One `CompiledBuild` may contain a `SchedulePack` keyed by `RuntimeMode` rather than a single `GbSchedIR`. All modes share the same artifact semantics, checkpoint schema, and continuation ABI. The runtime may switch modes only at declared safe boundaries (legal switch points). This is the right place for schedule multiversioning because `CompiledBuild` is explicitly replaceable and not part of artifact identity — interactive typing, steady-state generation, and trace-heavy debugging want different tradeoffs in tile sizes, yield spacing, kernel residency, common-bank pressure, and trace density.

This is where the compiler decides that a long expert matvec becomes many smaller tiles and inserts safe yield points between them, and where each slice is proven to have a bounded worst-case interrupt latency. Slices also participate in the liveness contract — every slice is expected to make measurable progress at the `SemanticCheckpointId` level.

`ResourceStateValidation` runs after `GbSchedIR` and proves:
- all resource leases are balanced (every `AcquireLease` has a matching `ReleaseLease`),
- no illegal yield crosses a non-resumable lease,
- no ISR-visible path depends on leased switchable state,
- overlay and bank-shadow assumptions match the slice's declared residency.

Every build emits `certs/resource_state.cert.json`.

### 11. `ScheduleCostAnalysis`

`ScheduleCostAnalysis` is the single load-bearing producer of objective-facing cost envelopes. It runs over `GbSchedIR` / `SchedulePack`, uses calibration bundles, and produces per-mode cost envelopes that predict objective satisfaction and feed the refinement loop.

```rust
pub struct ScheduleCostReport {
    pub objective: CompileObjective,
    pub per_mode: BTreeMap<RuntimeMode, EstimatedCostDelta>,
    pub refs: Vec<EvidenceRef>,
}
```

### 12. Backend (`AsmIR -> ReachabilityValidation -> PlacedRom -> EncodedRom`)

The backend is a single headline step that contains **four** internal sub-passes.

#### `AsmIR`

This is your typed LR35902 eDSL plus pseudo-ops. It is the only authoring layer for generated executable code, but it should now be aware of:

- section roles,
- residency classes,
- pseudo-ops for yield, trace, and bank-lease acquisition,
- explicit provenance,
- cycle annotations,
- profile tags.

Legal instruction shapes and higher-level pseudo-ops such as `BankLease`, `BankRelease`, `FarCall`, `Yield`, `TraceProbe`, `AssertBank`, and `Db` / `Dw` data directives all live here. Raw byte blobs should be limited to the cartridge header and a few tightly audited escape hatches. Compiler-generated code may **not** emit raw MBC writes directly; it must go through the `BankLease`/`BankGuard` ABI in `gbf-runtime::banking`.

#### `ReachabilityValidation`

Before final placement, run a whole-program reachability and privilege analysis over `AsmIR` and the legalized call/branch/thunk edge graph. `SectionRole` annotations and residency rules are declarations; `ReachabilityValidation` turns them into proofs. It computes the transitive reachability classes of code/data after far-call legalization and thunk insertion:

- **ISR-reachable**,
- **yield-resume reachable**,
- **fault-path reachable**,
- **harness-entry reachable**,
- **bank-lease protected**,
- **normal only**.

Then it validates:

- ISR-reachable code/data is Bank0/HRAM/fixed-WRAM only,
- no forbidden MBC writes on privileged paths,
- no illegal `MachineEffect` on paths whose `PrivilegeClass` forbids it,
- no switchable-bank dependency on ISR or resume paths,
- no illegal reentrancy through bank guards,
- no unreachable continuation targets,
- no fault path that depends on non-resident data.

This pass **computes, rather than trusts**, which code/data must be Bank0/HRAM/fixed-WRAM only and which paths may legally depend on switchable residency. It catches exactly the sort of subtle banked-runtime bugs that would otherwise survive until late emulator or hardware testing.

#### `PlacedRom`

This is after layout, banking, label resolution, branch expansion, and far-call legalization. `PlacedRom` owns placement profiles explicitly:

- `StrictOnePerBank`: simplest bring-up and debugging profile;
- `Budgeted`: default profile; one expert section may use only part of a bank and must leave reserved slack;
- `PackedExperts`: multiple small experts or expert fragments may co-reside when legal.

The layout/legalization stage is sophisticated:

- branch relaxation,
- far-call thunk insertion,
- bank-switch coalescing,
- placement by profile-guided expert hotness (from `ExportFacts` / `CompilePreferences`),
- stable symbol naming,
- deterministic section ordering,
- common-bank versus expert-bank partitioning,
- enforcement of residency decisions from `RomWindowPlan`,
- enforcement of the ISR residency rule (proven, not declared, by `ReachabilityValidation`).

This stage owns the global constraints:

- no section crosses a bank boundary,
- all relative branches are in range or rewritten,
- all expert sections satisfy residency rules,
- all SRAM/WRAM arenas fit,
- all continuation targets are valid and reachable,
- bank packing is deterministic.

#### `EncodedRom`

`EncodedRom` remains intentionally boring. It emits bytes only after all high-level decisions are already frozen:

- `.gb`
- `.sym`
- `.lst`

The encoder should be tiny.

### 13. `BuildReports`

Every build emits a complete report package consumed by tests, dashboards, and the day-to-day debugging flow: `map.json`, `provenance.json`, `budget.json`, `slice_report.json`, `trace_schema.json`, `conformance.json`, `oracle_vectors.bin`, `semantic_checkpoint_schema.json`, `operational_probe_schema.json`, `artifact_lineage.json`, `workload_manifest.json`, `hint_consumption.json`, `compiler_feedback.json`, `reachability_report.json`, and (under `Trace` builds or any time the `StageCache` is cold) `stages/` — serializable snapshots for every transformative pass. Every build also emits `trace_perturbation_report.json` and, when tracing is enabled, `trace_loss_report.json`. Critical passes also emit machine-checkable certificates under `certs/`: `certs/range.cert.json`, `certs/arena.cert.json`, `certs/window.cert.json`, `certs/reachability.cert.json`. See the Reports and artifacts section below.

## The runtime architecture

The runtime remains a cooperative kernel centered on Bank0, but the bank partitioning is now more deliberate and less Bank0-heavy, and ROM residency is resolved by `RomWindowPlan` and proven by `ReachabilityValidation` rather than assumed.

**Additional hard rule: all ISR code and ISR-reachable data live in Bank0, HRAM, or fixed WRAM only. Interrupt handlers may not depend on the currently selected switchable ROM or SRAM bank.** This is computed by `ReachabilityValidation`, not declared and hoped for. It is the difference between "my cooperative runtime is cooperative" and "my runtime locks up on cartridge after twenty minutes because bank shadow state and hardware bank state diverged across an interrupt boundary."

Bank classes:

- **`Bank0 / RuntimeNucleus`**
  - interrupt vectors and boot/header glue,
  - scheduler,
  - joypad,
  - text renderer,
  - keyboard,
  - **video commit queue**,
  - animation state,
  - panic/debug screen,
  - far-call trampolines,
  - tiny hot dispatch stubs,
  - **expert hot kernels that must stream expert-bank data** (when `RomWindowPlan` selects `Bank0Fixed`),
  - all ISR-reachable code and data (proven by `ReachabilityValidation`),
  - optional dev harness entry.

- **`CommonBanks`**
  - shared kernels and orchestrator slices whose **code and data co-reside** in the same visible bank,
  - embeddings,
  - router weights/tables,
  - classifier head,
  - shared LUTs,
  - common constants.

- **`ExpertBanks`**
  - expert-local tensor payloads,
  - tiny expert entry stubs,
  - expert-local metadata,
  - optional expert-local LUT fragments.

The default kernel strategy becomes:

- expert hot kernels execute from Bank 0 or a WRAM overlay when they must stream expert-bank data;
- switchable common banks are reserved for phases whose code and data co-reside in the same visible bank;
- expert-local tensor payloads live in expert banks;
- tiny expert entry stubs live in each expert section;
- no giant duplicated fully-unrolled expert code unless a profile proves it wins.

MBC5 gives you the fixed bank plus a switchable 16 KiB window, and up to 8 MiB total ROM, so this split is natural. ([gbdev.io][2])

`gbf-runtime` should be authored as Rust builders over `AsmIR`. Its core modules are:

- `boot`
- `interrupts`
- `scheduler`
- `banking` (owns the `BankLease` / `BankGuard` ABI — the only legal path to MBC writes)
- `joypad`
- `text`
- `keyboard`
- `video_commit` (owns the `UiCommitPlan` and the video commit queue)
- `panic`
- `trace`
- `harness`
- `persistence`

## Memory plan

Memory hierarchy is explicitly tied to `RomWindowPlan`, the scheduler, and the arena plan.

```text
ROM bank 00 (fixed) — RuntimeNucleus
  boot/header/vectors
  ISR + scheduler                 ; ISR-reachable code/data only (proven)
  UI + keyboard + text renderer
  video commit queue
  font/assets
  panic/debug screen
  resume/far-call trampolines
  expert hot kernels that stream expert-bank data

ROM banks 01..K — CommonBanks
  co-resident shared kernels and orchestrator slices
  embedding tables
  router weights/tables
  classifier head
  shared LUTs and common constants

ROM banks K+1..N — ExpertBanks
  expert-local weights
  expert-local row/tile metadata
  tiny expert entry stubs
  optional expert-local LUTs
```

WRAM hot arena:

- activation ping-pong buffers,
- accum scratch,
- route scratch,
- decode scratch,
- continuation record (including liveness fields),
- call stack,
- small live temporaries that survive a slice boundary.

WRAM overlay (optional, profile-controlled, allocated by `RomWindowPlan`):

- copied hot micro-kernels,
- tiny staged LUT fragments,
- bank-switch thunks when Bank 0 space is too tight.

HRAM fast flags:

- frame flags,
- current ROM/SRAM bank shadow registers,
- last fault code,
- tiny ISR scratch,
- ultra-hot scheduler fields.

SRAM persistent arena:

- double-buffered persistent sequence state pages (KV or linear state),
- prompt/output scrollback / transcript history,
- dev trace pages,
- harness command/result blocks,
- large cold spills only when unavoidable.

VRAM / OAM:

- UI-owned only; generated inference slices are forbidden from touching them. All VRAM/OAM commits go through the `video_commit` module's queue against legal LCD modes.

The key revision to the earlier yield story is this:

- default yield: save the continuation record in WRAM and return to Bank0;
- sequence state is persisted in SRAM because it is model state, not because yielding demands it;
- large SRAM spills are allowed, but they are a deliberate compiler choice, not the default context-switch mechanism.

In other words: SRAM is persistent state first, oversized spill area second, and "save everything here every yield" never.

Because the visible SRAM window is only 8 KiB at a time, the compiler should treat SRAM as paged persistent storage. Hot tiles get copied into WRAM, worked on there, and written back explicitly. Do not let hot loops stream randomly through banked SRAM.

For MBC5 register handling, I would centralize ROM-bank writes, SRAM-bank writes, and RAM enable/disable in the `gbf-runtime::banking` module, keep software shadows in HRAM, and expose bank changes only through a small `BankLease` / `BankGuard` ABI that couples hardware writes, shadow updates, and any required short critical section. Pan Docs notes the canonical RAM-enable value is `$0A`, and that relying on other low-nibble-`A` values is not recommended for compatibility. ([gbdev.io][2])

### Persistent record protocol

Battery-backed SRAM is exactly the kind of state that becomes painful when you mix "model state," "UI transcript," "harness I/O," and "resume state" without a durable record protocol. Every persistent SRAM record carries a small header that encodes both its durability class and its torn-write state:

```rust
pub struct PersistHeader {
    pub magic: [u8; 4],
    pub kind: PersistKind,
    pub page_state: PageState,
    pub state_schema: u16,
    pub artifact_hash: Hash128,
    pub semantic_state_hash: Option<Hash256>,
    pub resume_abi_hash: Option<Hash256>,
    pub build_identity_hash: Option<Hash256>,
    pub generation: u32,
    pub durability: DurabilityClass,
    pub checksum: PersistChecksum,
}

pub struct PersistGroupCommit {
    pub id: CommitGroupId,
    pub generation: u32,
    pub member_mask: u16,
    pub checksum: PersistChecksum,
    pub commit_word: u16,
}

pub enum PersistKind {
    SequenceState,
    Continuation,
    Transcript,
    Harness,
    Trace,
}

pub enum DurabilityClass {
    Critical,
    Recoverable,
    BestEffort,
}

pub enum PersistChecksum {
    Fletcher16(u16),
    Crc32(u32),
}

pub enum PageState {
    Writing,
    Committed,
    Retired,
}
```

Rules:

- sequence state is stored in double-buffered (or ring-buffered) pages with explicit page states (`Writing -> Committed -> Retired`) and a commit word written last so torn writes are detectable;
- writes to persistent state happen only at explicit commit boundaries (`NeedInput`, `TokenReady`, `Finished`, or a compiler-approved checkpoint);
- HRAM may cache the active page for runtime speed, but authoritative recovery
  state is always encoded in SRAM headers / commit records;
- pages that must remain mutually consistent (for example sequence state + transcript delta + token output) are assigned the same `CommitGroupId`, and the small `PersistGroupCommit` manifest is written last;
- boot resumes only the newest fully committed group, never a best-effort mixture of individually valid pages from different epochs;
- boot validates the newest committed page via CRC, commit word, and record-specific compatibility rules:
  `SequenceState` validates `semantic_state_hash`,
  `Continuation` validates `resume_abi_hash`,
  and harness/trace pages validate `build_identity_hash`;
  boot cold-starts only the incompatible record families instead of invalidating all persisted state together;
- transcript, harness, and trace data use distinct page families so recovery policy is record-specific rather than globally coupled — a corrupt trace page must never contaminate sequence-state recovery;
- compatible schema upgrades may register an explicit `StateMigrator`; absence of a migrator remains a clean cold start;
- `gbf-runtime::persistence` owns header layout, CRC verification, and page rotation.

Default behavior is conservative: absent a valid committed page or explicit migrator, cold-start cleanly. Upgrades become less user-hostile without forcing migration complexity into the common path, and debugging gets easier because the persistence layer is a state machine, not a bag of conventions.

## Auto-yielding without hanging the UI

This is designed as a coroutine ABI with deadline-assisted yielding, not as a hope.
The compiler inserts safe points; the runtime may arm a soft deadline using
TIMA, and the timer ISR sets a `yield_requested` flag in HRAM. Generated code
polls that flag only at declared safe points. The types live in `gbf-abi` with explicit `AbiVersion` and `#[repr(C)]` layouts so compiler, runtime, harness, emulator adapters, and `ScheduleOracle` cannot drift. The ABI also carries an explicit **liveness contract**: a cooperative runtime can be locally safe and globally broken (livelock, oscillation between tiny slices with no semantic progress, repeatedly revisiting the same checkpoint, starving generation while the UI runs smoothly). Liveness is not optional.

```rust
// in gbf-abi
#[repr(C)]
pub struct InferenceState {
    pub cont_slice: SliceId,
    pub cont_bank: RomBank,
    pub cont_addr: u16,
    pub phase: Phase,
    pub layer: u8,
    pub expert: u8,
    pub arena_cursor: ArenaCursor,
    pub sram_bank: u8,
    pub input_token: u8,
    pub output_token: u8,
    pub rng_state: u16,
    pub flags: RuntimeFlags,
    pub error_code: u8,
    pub progress_epoch: u32,
    pub last_checkpoint: CompactCheckpointId,
    pub no_progress_frames: u16,
}

#[repr(C)]
pub struct HarnessCommandBlock {
    pub abi_version: u16,
    pub op: HarnessOp,
    pub arg0: u32,
    pub arg1: u32,
    pub payload_len: u16,
    pub payload_ptr: u16,
    pub flags: u16,
}

#[repr(C)]
pub struct HarnessResultBlock {
    pub abi_version: u16,
    pub status: FaultCode,
    pub result_kind: HarnessResultKind,
    pub payload_len: u16,
    pub payload_ptr: u16,
    pub checkpoint_count: u16,
}

pub enum HarnessOp {
    RunPrompt,
    StepSlice,
    RunUntilCheckpoint,
    DumpArena,
    DumpPersistentPage,
    QueryIdentity,
    InjectFault,
    PowerCut,
}

pub enum HarnessResultKind {
    OutputBytes,
    CheckpointDump,
    ArenaDump,
    PersistentDump,
    Identity,
    FaultSnapshot,
}

pub enum SemanticCheckpointId {
    PostEmbedding { layer: LayerId },
    PostRouter { layer: LayerId },
    PostExpertDowncast { layer: LayerId, expert: ExpertId },
    PostLogits,
    PostDecode,
}

pub enum FaultCode {
    Ok,
    Bounds,
    BankShadowDivergence,
    ContinuationCorrupt,
    InterruptOverrun,
    LivenessTimeout,
    RepeatedCheckpointNoProgress,
    TraceBudgetExceeded,
    CalibrationDrift,
    ObjectiveViolation,
    // ...
}

pub enum FaultDomain {
    Semantic,
    Residency,
    Persistence,
    Observability,
    Timing,
    Harness,
}

pub enum RecoveryAction {
    RetrySlice,
    DemoteMode(RuntimeMode),
    DropTrace,
    ColdStartRecordFamily(PersistKind),
    PanicScreen,
    HardReset,
}

pub struct FaultRecoveryRule {
    pub fault: FaultCode,
    pub action: RecoveryAction,
    pub escalate_after: u8,
}

pub struct FaultPolicy {
    pub rules: Vec<FaultRecoveryRule>,
}

pub struct CompatibilityEnvelope {
    pub abi: AbiVersionRange,
    pub artifact_features: BTreeSet<ArtifactFeature>,
    pub compiler_features: BTreeSet<CompilerFeature>,
    pub runtime_features: BTreeSet<RuntimeFeature>,
    pub harness_features: BTreeSet<HarnessFeature>,
}

#[repr(C)]
pub struct BuildIdentityBlock {
    pub abi_version: u16,
    pub artifact_core_hash: Hash256,
    pub lowering_hash: Hash256,
    pub compile_request_hash: Hash256,
    pub runtime_nucleus_hash: Hash256,
    pub calibration_hash: Option<Hash256>,
    pub target_profile: TargetProfileId,
    pub compatibility_hash: Hash256,
}

#[repr(C)]
pub struct FaultSnapshot {
    pub fault: FaultCode,
    pub domain: FaultDomain,
    pub recommended_action: RecoveryAction,
    pub slice: SliceId,
    pub rom_bank: RomBank,
    pub checkpoint: CompactCheckpointId,
    pub progress_epoch: u32,
    pub flags: RuntimeFlags,
}

pub enum PersistScanPolicy {
    StrictCriticalOnly,
    ScanAll,
}

pub struct BootValidationPlan {
    pub persist_scan: PersistScanPolicy,
    pub startup_mode: RuntimeMode,
    pub verify_identity_block: bool,
}
```

`CompatibilityEnvelope` declares the feature/version ranges a build expects, so runtime, harness, emulator adapter, persistence layer, reports, hardware dumps, and bug reports can all point at the same contract. `BuildIdentityBlock` is a ROM-resident compact identity structure queryable by harness and visible in dumps — once you have schedule packs, richer harness ops, multiple calibration cohorts, and persisted state, "what exactly am I looking at?" needs a single authoritative answer. `FaultSnapshot` is captured on fault and gives the fastest path to useful diagnosis on real hardware.

`SemanticCheckpointId`s are derived from canonical model paths, not incidental pass-local numbering. Compact numeric ids (`CompactCheckpointId`) may exist in ROM for space, but every external report maps them back to these stable symbolic ids.

The continuation record in WRAM is minimal: just enough for precise coroutine resumption plus the liveness counters. `progress_epoch` is monotonically advanced every time the runtime passes a `SemanticCheckpointId`; `last_checkpoint` records the most recent id reached; `no_progress_frames` counts frames since the last `progress_epoch` advance.

Yield points are still inserted only at stable checkpoints:

* current tile complete,
* live registers spilled,
* bank shadow updated,
* temporary results committed,
* continuation target written.

The scheduler in Bank0 then works like this:

1. VBlank ISR sets a frame flag and performs only tiny, well-saved housekeeping.
2. Main loop consumes input, advances spinner/caret state, and commits UI dirty regions during approved UI windows via the video commit queue.
3. Main loop resumes inference slices until the reserved compute budget (adjusted for the current `UiCommitPlan`) is used.
4. When idle, Bank0 can sleep with `halt`; Pan Docs notes that `halt` wakes when an interrupt is pending, with important caveats when `IME=0`, so it belongs in the runtime scheduler, not in generated inference slices. ([gbdev.io][4])

The scheduler policy is refined and expressed as explicit data:

```rust
pub struct FrameBudget {
    pub compute_cycles: CycleBudget,
    pub trace_bytes: u16,
    pub persist_bytes: u16,
    pub overlay_installs: u8,
}

pub struct SchedulerPolicy {
    pub hard_ui_reserve: CycleBudget,
    pub soft_ui_reserve: CycleBudget,
    pub video_commit_margin: CycleBudget,
    pub max_slice_cycles: CycleBudget,
    pub adaptive_headroom: u16,
    pub max_interrupt_latency: CycleBudget,
    pub soft_deadline_margin: CycleBudget,
    pub max_trace_bytes_per_frame: u16,
    pub max_persist_bytes_per_frame: u16,
    pub max_overlay_installs_per_frame: u8,
    pub max_no_progress_frames: u16,
    pub min_progress_events_per_resume_window: u8,
    pub runtime_mode: RuntimeMode,
}

pub enum YieldCheckClass {
    None,
    PollHramFlag,
    DeadlineAware,
}

pub struct UiCommitPlan {
    pub dirty_tiles: u16,
    pub dirty_oam: bool,
    pub estimated_cycles: CycleBudget,
    pub latest_safe_mode: LcdMode,
}
```

Policy rules:

- budget compute **per frame**, not "per VBlank" by name;
- reserve an explicit `hard_ui_reserve` before running inference slices and a `soft_ui_reserve` for adaptive backoff;
- **derive the effective compute budget from the current `UiCommitPlan`**, so actual dirty-region pressure directly reduces inference budget rather than living behind a fixed scalar;
- allow inference during visible scanlines so long as slices never touch VRAM/OAM;
- keep UI dirty-region commits under Bank0 `video_commit` ownership only;
- no slice runs whose worst-case interrupt latency exceeds `SchedulerPolicy::max_interrupt_latency` without a matching `InterruptPolicy::ShortCriticalSection` or `Disabled` justification;
- if `no_progress_frames` exceeds `max_no_progress_frames`, raise `FaultCode::LivenessTimeout`;
- if the same `last_checkpoint` is revisited without intervening progress beyond a threshold, raise `FaultCode::RepeatedCheckpointNoProgress`.

I would formalize three resume classes:

- `resume_micro_slice()` for very small slices,
- `resume_frame_slice()` for normal cooperative inference,
- `resume_blocking_boundary()` for token-ready / need-input / finished transitions.

The main loop is therefore:

- poll frame/input flags,
- advance UI animation,
- compute a `UiCommitPlan` and commit UI dirty regions against legal LCD modes,
- run as many inference slices as fit inside the reserved compute budget (minus `UiCommitPlan::estimated_cycles`),
- stop before UI reserve is endangered,
- update liveness counters,
- repeat.

Start with conservative static worst-case slice ceilings (a few hundred to around a thousand M-cycles), then move to an **adaptive policy** once emulator and hardware measurements exist: when UI load is light, allow longer slices; when dirty-region pressure or input activity rises, shrink the slice ceiling preemptively. Since a frame is about 17.5k M-cycles and VBlank alone is only about 1140 M-cycles, the engine should do most of its math during the visible frame while simply avoiding any direct VRAM/OAM traffic. UI code then owns the video memory contract. ([gbdev.io][5])

Generated inference slices are forbidden from touching VRAM/OAM directly. That single rule is what lets the UI remain smooth while inference runs.

## Assembly eDSL (`gbf-asm`)

`gbf-asm` should be treated as a full authoring/debug surface, not just "typed instructions + encoder". It owns:

- typed LR35902 instructions,
- pseudo-ops,
- data directives,
- section roles,
- residency classes,
- instruction provenance,
- a cycle model,
- pretty-printing,
- layout support,
- branch relaxation,
- symbol generation,
- final encoding.

Add section-level typing:

```rust
pub enum SectionRole {
    RuntimeBank0,
    IsrReachable,
    CommonKernel,
    CommonWeights,
    ExpertPayload(ExpertId),
    ConstData,
    TraceOnly,
}
```

And the higher-level pseudo-ops:

```rust
pub enum PseudoOp {
    BankLease(RomBank),
    BankRelease,
    FarCall { bank: RomBank, target: LabelId },
    Yield { kind: YieldKind },
    TraceProbe(TraceId),
    AssertBank(RomBank),
}

pub enum MachineEffect {
    RomBankWrite,
    SramBankWrite,
    RamEnableWrite,
    InterruptMaskChange,
    VramAccess,
    OamAccess,
}

pub enum PrivilegeClass {
    Unprivileged,
    BankingOnly,
    InterruptControl,
    VideoCommitOnly,
}
```

Provenance should be rich. Every instruction or data directive should be able to point back to:

- originating `GbNode` / `InferOp`,
- originating graph node / tensor id,
- lowering pass,
- section role,
- optional trace id,
- cycle estimate before and after legalization.

Every emitted instruction / pseudo-op should also carry a declared `MachineEffect` set and a `PrivilegeClass`, so `ReachabilityValidation` validates against explicit machine effects rather than reconstructing them after lowering.

`Raw(Vec<u8>)` remains legal only as an audited escape hatch for cartridge header bytes, tiny frozen micro-blobs, and test fixtures. It is not a general-purpose authoring mechanism.

### Profiles and objectives

Canonical first-wave profiles:

- `Bringup`
- `Default`
- `Trace`
- `Recovery`

Optional later-derived profiles, only when measurement justifies them:

- `Speed`
- `Size`

Each profile changes placement policy, observation/probe density, trace insertion, branch density tradeoffs, thunking strategy, whether certain kernels are cloned or shared, and which reports are mandatory.
Each profile also declares whether observability must preserve the compiled behavior class or may freely perturb it.

```rust
pub enum ObservabilityMode {
    Invariant,
    Flexible,
}

pub struct TraceBudget {
    pub max_events_per_slice: u16,
    pub max_bytes_per_frame: u16,
    pub drop_policy: TraceDropPolicy,
}
```

`ObservabilityMode::Invariant` means probes must preserve schedule/layout decisions within declared tolerances,
and the compiler must prove that claim with a paired-build comparison on a declared workload slice.
`ObservabilityMode::Flexible` means richer instrumentation may change the build.

```rust
pub struct PerturbationSummary {
    pub changed_slices: u16,
    pub changed_epochs: u16,
    pub changed_bank_placements: u16,
}

pub struct ObservabilityCertificate {
    pub mode: ObservabilityMode,
    pub compared_builds: [Hash256; 2],
    pub workloads: Vec<WorkloadId>,
    pub perturbation: PerturbationSummary,
    pub accepted: bool,
}
``` If invariant mode cannot be satisfied, the compiler should say so instead of silently emitting a materially different build and calling it "trace".

Each profile is also paired with a `CompileObjective` declared in `configs/compile/*.toml`, so profile selection and autotuning optimize against explicit success criteria rather than an informal notion of "faster". `CompileObjective` and profile id travel as part of the `CompileRequest`, not as part of the artifact.

## Types, passes, and tests: where each invariant lives

The correctness loop should be treated as **five** layers:

1. type-level local legality,
2. analysis-pass physical legality,
3. **denotational-to-artifact conformance**,
4. **artifact-to-schedule operational equivalence**,
5. hardware smoke and performance validation.

Concretely:

Types should cover things like `RomBank`, `SramBank`, `AddrSpace`, `TensorId`, `ExpertId`, `QFormat`, `CycleBudget`, `SectionRole`, `Residency`, `KernelResidency`, `KernelSpecId`, `StorageClass`, `LifetimeClass`, `CommitGroupId`, `Materialization`, `InterruptPolicy`, `RuntimeMode`, `SemanticCheckpointId`, `CompactCheckpointId`, `WorkloadClass`, `ObservabilityMode`, and legal LR35902 instruction forms. A value should know its numeric format and effect class; a storage binding should know whether it is recomputed, materialized (and its `LifetimeClass`), or persisted (and its `CommitGroupId`).

Compiler passes should prove things types cannot know locally:

- accumulator range safety (`RangePlan`),
- observation contract stability (`ObservationPlan`),
- storage class / persistence / alias safety (`StoragePlan`),
- arena layout feasibility (`ArenaPlan`),
- ROM window visibility and kernel residency (`RomWindowPlan`),
- whole-program reachability and privilege (`ReachabilityValidation`),
- section fits within assigned bank,
- no cross-bank expert spill,
- valid far-call lowering,
- relative branch legality,
- continuation safety at yield points,
- bounded worst-case interrupt latency per slice,
- bounded liveness (no-progress frames under threshold),
- deterministic section ordering and symbol generation.

`gbf-test` should organize the test matrix explicitly.

### Unit tests

- encoder opcode/operand tests,
- ternary pack/unpack tests,
- **independent reference pack/unpack vs production packer**,
- LUT generation tests,
- **independent logical LUT evaluator vs production LUT generator**,
- reduction-plan tests,
- range-analysis tests,
- observation-plan stability tests,
- storage-class / materialization / alias-class tests,
- arena-allocation tests,
- ROM window / kernel residency tests,
- reachability validation tests on synthetic call graphs,
- interrupt-policy / bank-lease tests,
- budget estimator tests,
- yield-insertion tests on toy blocks.

### Golden compiler tests (snapshot-based)

- artifact -> `QuantGraph` snapshots,
- `QuantGraph` -> `GbInferIR` (value/effect IR) snapshots,
- `GbInferIR` -> `ObservationPlan` snapshots,
- `ObservationPlan` -> `RangePlan` snapshots,
- `RangePlan` -> `StoragePlan` snapshots,
- `StoragePlan` -> `GbSchedIR` snapshots,
- `RomWindowPlan` snapshots,
- `ReachabilityValidation` classification snapshots,
- assembly listing snapshots,
- bank map snapshots,
- stable symbol naming.

### Property tests

- randomized tiny tensors,
- randomized prompts against tiny models,
- branch-relaxation invariants,
- pack/unpack round-trips (including `TargetDataLoweringArtifact` ↔ `ArtifactCore` round-trip),
- storage-plan alias-class invariants.

### Oracle tests (three-stratum)

- `DenotationalOracle` evaluates reference semantics on deterministic prompts **when a `ReferenceObservationCache` is present**;
- `ArtifactOracle` exact-evaluates the exported artifact in canonical logical form;
- `ScheduleOracle` evaluates `GbSchedIR` with matching arenas and continuation ABI;
- **denotational vs artifact conformance stays within the declared error envelope** recorded in `ConformanceEnvelope` / `conformance.json`;
- accumulator max matches budget reports;
- same packed tensor bytes reproduce same outputs across platforms under any given `TargetDataLoweringArtifact`.

### Differential tests (harness mode)

Harness mode is mandatory. The semantic correctness loop must not depend on driving the UI path for every test. In harness mode:

- prompt bytes go into a `HarnessCommandBlock` in SRAM,
- the ROM runs headlessly,
- checkpoints and final outputs are written to `HarnessResultBlock` + result pages,
- the harness compares those bytes against `DenotationalOracle`, `ArtifactOracle`, and `ScheduleOracle` at stable `SemanticCheckpointId`s.

The differential ladder is explicit:

- **denotational oracle vs artifact oracle** at stable checkpoint IDs and named quality metrics (quality/degradation envelope recorded in `conformance.json`; catches export/quantization regressions);
- **artifact oracle vs schedule oracle** at the same checkpoint IDs (catches scheduler/storage/arena regressions);
- **schedule oracle vs emulator/harness** at the same checkpoint IDs (catches backend/layout/reachability regressions);
- **emulator/hardware vs schedule oracle** at token boundaries and selected intermediate checkpoints (catches emulator-accuracy and real-hardware-only bugs).

That gives you checkpoint-level diffing like:

- post-embedding (`SemanticCheckpointId::PostEmbedding`),
- post-router,
- post-expert-downcast,
- post-logits,
- post-decode.

Per-layer buffer diffs, token-by-token output diffs, and trace-page diffs for debug builds all live here. That is how you localize bugs fast — and which stratum the bug belongs to tells you immediately which team of changes to bisect. Interactive UI tests remain separate and valuable, but they should not be the main semantic validation mechanism.

### UI smoke tests

- virtual keyboard navigation,
- prompt commit,
- animation while inference is active,
- no-hang frame progression,
- `UiCommitPlan` pressure tests (high dirty-region churn, OAM bursts).

### Liveness stress tests

- long-running generation with watchdog assertions on `progress_epoch`,
- explicit `LivenessTimeout` fault injection,
- repeated-checkpoint fault injection,
- starvation tests that keep UI busy and check inference still advances under `min_progress_events_per_resume_window`.

### Nightly trust tests

- stricter emulator backend,
- longer prompt suite driven by versioned `WorkloadManifest`s,
- performance regression gates against `CompileObjective` satisfaction,
- **common-mode failure checks comparing production and independent reference implementations on randomized artifacts**,
- liveness stress runs with watchdog assertions and no-progress fault injection,
- optional real-hardware smoke run.

The emulator strategy should be split in two on purpose:

- **fast backend** for bulk CI,
- **accurate reference backend wrapped from an external emulator/debugger** (SameBoy or BGB).

`gbf-emu` is an adapter layer around existing emulators, not a place to grow two emulator cores in-tree. ([GitHub][6])

## Reports and artifacts

Every build should emit a complete debugging/report package:

- `game.gb`
- `game.sym`
- `game.lst`
- `build_manifest.json`
- `map.json`
- `provenance.json`
- `budget.json`
- `policy_resolution.json`
- `calibration_resolution.json`
- `repair_report.json`
- `schedule_cost.json`
- `slice_report.json`
- `trace_schema.json`
- `metrics.json`
- `conformance.json` (denotational ↔ artifact deltas per workload)
- `oracle_vectors.bin`
- `semantic_checkpoint_schema.json`
- `operational_probe_schema.json`
- `artifact_lineage.json`
- `run_manifest.json`
- `workload_manifest.json`
- `hint_consumption.json`
- `compiler_feedback.json`
- `reachability_report.json`
- `stages/` (Trace builds or cold `StageCache`): serializable snapshots for every transformative pass
- `trace_perturbation_report.json`
- `observability.cert.json`
- `trace_loss_report.json` (when tracing is enabled)
- `failure_capsule.gbfz` (on first failing workload/checkpoint, followed by a best-effort reducer that emits `minimized_workload.toml` and `reduction_log.json` when it can shrink the case)
- `certs/range.cert.json`
- `certs/arena.cert.json`
- `certs/window.cert.json`
- `certs/sram.cert.json`
- `certs/resource_state.cert.json`
- `certs/reachability.cert.json`

`budget.json` should include:

- max WRAM / HRAM / SRAM usage,
- per-bank occupancy,
- slice histograms,
- estimated cycles per slice,
- estimated cycles per token,
- **observed vs predicted cycles per slice and per token** (once measurements exist),
- **evidence class and uncertainty envelope for every load-bearing estimate**,
- **reasons a decision was accepted under transferred or heuristic evidence**,
- **time-to-first-token distribution**,
- **max checkpoint-gap distribution**,
- **resume-latency distribution after input or wakeup**,
- **frame-jitter distribution under interactive workloads**,
- **cycle-model confidence / calibration age**,
- **compile-objective satisfaction at the requested quantiles**,
- **fallback reason when calibration confidence is insufficient**,
- **compile-objective satisfaction by workload**,
- **scheduler headroom utilization**,
- **video-commit cost distribution vs `video_commit_margin`**,
- worst-case interrupt latency per slice and aggregated,
- liveness margin (distance to `max_no_progress_frames`),
- predicted bank switches per token,
- predicted SRAM page switches per token,
- predicted yields per token,
- observed bank switches per token,
- observed SRAM page switches per token,
- observed timer-preempt requests,
- observed trace drops,
- overlay install counts,
- common-bank footprint,
- ROM window / kernel residency summary,
- expert hotness / placement notes,
- reduction-plan decisions.

Treat these as first-class regressions, not just debugging trivia.

When a mismatch occurs, the failure triage order should be explicit and aligned with the semantic strata:

1. reference/denotation mismatch (reference observation cache itself changed or regressed),
2. artifact-core mismatch (semantic core hash / lowering round-trip),
3. denotational-to-artifact conformance failure (`conformance.json` outside envelope),
4. artifact oracle mismatch,
5. schedule oracle mismatch,
6. observation-plan / range-plan / storage-plan / arena-plan / window-plan mismatch,
7. reachability validation failure,
8. layout mismatch,
9. runtime-only mismatch (including liveness faults),
10. hardware-only mismatch.

## The day-to-day iteration flow

I would make the revised development loop explicit in three nested layers.

### Inner compiler/runtime loop

1. edit lowering / runtime / layout code,
2. run encoder and analysis unit tests,
3. run snapshot tests,
4. run tiny harness-mode differential tests,
5. inspect listing / map / provenance / reachability diff,
6. commit.

### Model/export loop

1. edit topology / quant / dataset config,
2. run `gbf-train preflight` against the current `DeployabilityEnvelope` before committing to a long run,
3. train or fine-tune with periodic shadow export/compile on the EMA weights,
4. choose a checkpoint from the training frontier, then export `ModelArtifact`,
5. run oracle suite (denotational when a `ReferenceObservationCache` is present, then artifact, then schedule),
6. compile with an explicit `CompileRequest`,
7. run medium harness-mode differential suite,
8. run UI smoke + liveness stress suite,
9. then hardware smoke.

### Tuning / optimization loop

1. export `HintBundle` (`ExportFacts` from training statistics, `CompilePreferences`, `BuildConstraints`) and ingest `compiler_feedback.json` from the previous build,
2. compile under `Bringup`, `Default`, and `Trace` profiles (and `Speed` / `Size` once measurements justify them), each paired with its `CompileObjective` and — when appropriate — a `CalibrationSetRef`,
3. compare bank occupancy, slice histogram, predicted switches/token, observed vs predicted cycles, cycle-model drift, compile-objective satisfaction, hint consumption report, liveness margin, and actual emulator timing,
4. when the frontier is unclear, run constrained autotune via `gbf-bench` over tile sizes, kernel residency, and slice ceilings on named workloads,
5. choose the winning `CompileRequest` (profile + objective + calibration),
6. only then consider architecture changes.

The compiler should support an **always-on content-addressed `StageCache`**, with `--resume-from <stage>` as the user-facing debugging control layered on top. Cache keys must include semantic core hash, data-lowering hash, compile-request hash, calibration id, pass version, and feature flags. Combined with the `stages/` directory under `Trace` builds, this turns many nasty bugs into ordinary bisect/debug problems, dramatically speeds iteration on layout/backend bugs, makes profile matrices cheap, and improves autotune throughput.

In command form, I would aim for something like:

```text
cargo run -p gbf-train -- preflight configs/model/moe_char.toml
cargo run -p gbf-train -- train configs/model/moe_char.toml
cargo run -p gbf-train -- export runs/last.ckpt artifacts/builds/model.gbf
cargo run -p gbf-cli   -- oracle artifacts/builds/model.gbf --stratum denotational --workload fixtures/workloads/daily.toml
cargo run -p gbf-cli   -- oracle artifacts/builds/model.gbf --stratum artifact   --suite fixtures/prompts/standard.txt
cargo run -p gbf-cli   -- compile artifacts/builds/model.gbf --request configs/compile/trace.toml
cargo run -p gbf-cli   -- compile artifacts/builds/model.gbf --request configs/compile/default.toml --resume-from GbSchedIR
cargo run -p gbf-bench -- autotune artifacts/builds/model.gbf --workload fixtures/workloads/daily.toml --calibration configs/calibration/sameboy.toml
cargo test -p gbf-test -- --nocapture
```

The debugging flow should also become explicit and maps directly to the semantic strata:

- if `conformance.json` degrades: the **denotational** stratum regressed — it is a model-quality / export-quantization problem, not a scheduler problem;
- if the artifact diff fails: the **artifact** stratum regressed — inspect `ArtifactOracle` vectors and checkpoint traces;
- if the schedule diff fails: the **operational** stratum regressed — inspect `ScheduleOracle` over `GbSchedIR` at the matching checkpoint;
- if layout or reachability fails: inspect placement, window, storage, arena, and `reachability_report.json`;
- if performance regresses: inspect slice report, bank-switch report, cycle-model drift, liveness margin, and observed-vs-predicted timing;
- if a liveness fault fires: inspect `progress_epoch` traces and `last_checkpoint` history;
- if only hardware fails: run trace build, dump trace pages, compare against emulator trace, check for interrupt-latency violations.

The important social rule is that every architecture change answers three questions:

* did denotational quality (not just artifact consistency) improve or hold?
* did ROM/state budgets still fit?
* did cycles/token, bank-switch count, worst-case interrupt latency, and liveness margin get better or worse?

If you cannot answer all three, the iteration is incomplete.

## What I would build first

Not the whole transformer. I would build the stack in milestone form:

- **M0**: `gbf-asm`, encoder, symbol map, ROM builder; `gbf-hw` target profiles + calibration schema; `gbf-abi` skeleton (including liveness fields); Bank0 UI/runtime skeleton with VBlank, keyboard, text output, video commit queue, cooperative scheduler, and `BankLease`/`BankGuard` ABI.
- **M1**: `DenotationalOracle` + `ArtifactOracle` plus a single quantized dense kernel; conformance checking between reference observations and the frozen artifact (first `conformance.json`); first `CompileRequest` wiring.
- **M2**: one shared micro-kernel resolved by `RomWindowPlan`, plus one expert payload bank, with exact emulator diffing against `ScheduleOracle` and checkpoint alignment against `ArtifactOracle` at `SemanticCheckpointId` boundaries; first `ReachabilityValidation` pass integrated into the backend.
- **M3**: top-1 router, expert dispatch table, value/effect `GbInferIR` + `ObservationPlan` + `RangePlan` + `StoragePlan` wired end-to-end for a routed FFN under the cooperative scheduler.
- **M4**: sequence-state block — bring-up under tiny `SequenceSemanticsSpec::BoundedKv`, then `LinearState` evaluated as an equal alternative (not assumed winner).
- **M5**: full interactive text-generation loop with prompt warmup, generation, UI output (with `UiCommitPlan` driving compute budget), versioned SRAM persistence protocol, and liveness stress tests in nightly.
- **M6**: profile-guided placement, optional training-time regularizers (`λ_distill`, `λ_balance`, `λ_zrouter`, `λ_switch`, `λ_range`, `λ_zero`, `λ_shape`, `λ_overflow`), adaptive scheduler policy, `gbf-bench` calibration/autotune loop emitting concrete layered calibration bundles (`PlatformCalibrationBundle`, `KernelCalibrationBundle`, `RuntimeCalibrationBundle`), optional `ExpertShapePolicy::StructuredWidthGates` research mode for adaptive bank-aware expert width, and performance tuning.

Explicit fallback paths are part of the architecture, not admissions of failure:

- if the recurrent/linear sequence block underperforms in quality, the bounded-KV profile is already a first-class path — no boundary change is needed;
- if full ternary hurts too much, keep router and classifier at higher precision in common banks;
- if exact one-expert-per-bank wastes too much ROM, switch placement profile without retraining;
- if mid-layer yields are too expensive, coarsen slice boundaries while keeping the same coroutine ABI;
- if the denotational-vs-artifact conformance envelope is unacceptable, raise quantization precision on selected pieces (router, classifier, hot norms) rather than re-defining success.

That order gives you a functioning compiler-runtime-verification loop before you pay the full complexity cost of the model.

## Engineering rules

1. All generated executable code originates from `AsmIR` / `Instr` / audited runtime builders, never from ad hoc byte pushes.
2. Only the encoder translates legal instructions to bytes.
3. Every instruction and data directive carries provenance.
4. Every hard fit is proven in analysis/layout passes, not guessed in lowering code.
5. Artifact builds and ROM builds are deterministic and hashed (`ArtifactCore` canonical payload hash + `TargetDataLoweringArtifact::lowering_hash` + `CompileRequest` hash); `ArtifactManifest` is not self-hashed recursively.
6. Every ROM build emits its full report pack, including `conformance.json`, `hint_consumption.json`, `compiler_feedback.json`, and `reachability_report.json`.
7. The harness uses symbols and `SemanticCheckpointId`s derived from canonical model paths, not magic addresses or incidental pass-local numbering.
8. The compiler owns yield insertion and coroutine legality.
9. Shared model/compiler contracts live in `gbf-artifact`; shared live-execution contracts live in `gbf-abi`; training internals do not leak into codegen.
10. `Raw(Vec<u8>)` remains an escape hatch, never the default path.
11. `gbf-hw`, `gbf-artifact`, `gbf-abi`, `gbf-ir`, and `gbf-asm` are `no_std + alloc` capable where practical.
12. `unsafe` is forbidden by default and isolated to tiny audited islands when unavoidable.
13. Harness, trace, and oracle contracts use stable `SemanticCheckpointId`s, not raw addresses.
14. **Terminology discipline.** Reserve **denotational** for target-independent meaning, **artifact-semantic** for canonical exported behavior, and **operational** for schedule/runtime behavior. Do not use "semantic" as a synonym for "exact executable reference." The five-product decomposition is an ownership/build decomposition; the three-stratum decomposition is the semantic decomposition; the four-layer object decomposition classifies every runtime entity as semantic identity, deterministic data lowering, compile request, or build product; none is a synonym for the others.
15. All ISR code/data is Bank0-resident and bank-agnostic, **proven** by `ReachabilityValidation`, not declared; compiler-generated code may not perform raw MBC writes outside the `BankLease`/`BankGuard` ABI.
16. Every slice has an explicit `InterruptPolicy` and a bounded worst-case interrupt latency proven against `SchedulerPolicy::max_interrupt_latency`; every slice also participates in the liveness contract via `progress_epoch` / `last_checkpoint` / `no_progress_frames`.
17. Verification-critical algorithms (pack/unpack, logical LUT evaluation, decode/RNG) have an independent slow reference implementation; nightly checks compare production against reference on randomized artifacts.
18. The frozen artifact is never allowed to silently redefine success: `conformance.json` is a first-class regression gate against a denotational reference.
19. `ArtifactCore` is target-independent; physical packings, reorders, packed LUT blobs, and bank-chunked layouts belong only in `TargetDataLoweringArtifact`s that must round-trip to the core. Target profile id, compile profile, calibration id, and any build-constraint overrides live in `CompileRequest`, not in `ArtifactManifest`.
20. The compiler supports an always-on content-addressed `StageCache` with two levels: shard-local keys derived from named component digests where legality is local, and whole-build keys where legality is global. `--resume-from <stage>` is layered on top of those caches rather than replacing them.
21. `WorkloadManifest` is strongly typed by purpose (`WorkloadClass`) and uses an `AcceptanceMatrix` with explicit per-stratum gates, not a single generic acceptance blob.
22. `HintBundle` splits cleanly into `ExportFacts` (measured evidence), `CompilePreferences` (cost-function nudges), and `BuildConstraints` (hard admissibility conditions); the compiler treats the three differently and reports consumption explicitly.
23. Critical safety passes emit machine-checkable certificates consumed by `gbf-verify`; human-readable reports are not the only source of truth for pass validity.
24. Deterministic builds require a pinned toolchain, lockfile, and host triple at minimum; every train/export/compile/bench run emits a minimal `ReproducibilityManifest`. Extra host-environment capture is opt-in diagnostic data until real nondeterminism justifies making it contractual.

## Bottom line

The architecture I would actually ship is **five cooperating products plus three shared contracts, with three semantic strata that cut across those products, four object-level layers that keep every runtime entity unambiguously one thing, and a measured calibration loop that closes the whole thing**.

The five products:

* Rust training and export, Burn-fronted as a training host (backend portability, autodiff, optimizers, checkpoints, metrics), with `gbf-model` owning deployable numeric semantics (ternary projection, activation fake-quant, norm approximation, export visitation) because built-in QAT is currently not supported; Burn-native PTQ serves as a baseline and ablation path; training proceeds in explicit `TrainPhaseSpec` phases (`DenseTeacherWarmup` → `RouterWarmup` → `ExpertTernaryQat` → `FullNumericQat` → `HardenAndSelect`), with a frozen dense teacher exported as `ReferenceModelBundle` and periodic shadow export/compile via `ShadowCompilePolicy` for Pareto frontier checkpoint selection; backend choice is an implementation detail behind the training/model boundary, pinned to an exact Burn version. ([Docs.rs][1])
* A versioned shared artifact boundary with an immutable `ArtifactCore` (canonical semantic, target-independent, quantized; `LexicalSpec` for model semantics, `DecodeCapabilitySet` for supported decode modes), zero or more deterministic `TargetDataLoweringArtifact` sidecars (physical packing, packing layouts, packed LUT blobs, kernel compatibility metadata per `DataLoweringProfileId`, with `compatible_targets` for cache reuse), a mutable `ArtifactAux` (golden vectors via `SidecarRef`, `ReferenceObservationCache` via `SidecarRef`, hierarchical `ConformanceEnvelope` via `SidecarRef`, `HintBundle`, `InteractionBundle`, `CompilerFeedback`), and an optional sibling `ReferenceModelBundle` (with `ReferenceNumericProfile`) for denotational truth — plus a separate `CompileRequest` boundary (with `CalibrationSetRef` for layered calibration) and a `ResolvedCompilePolicy` that captures final objectives, constraints, repair policy, and provenance before the transform pipeline begins, backed by a content-addressed `blobs/sha256/` store for efficient deduplication and lazy loading.
* **Three oracles**: a `DenotationalOracle` consuming the sibling `ReferenceModelBundle` to define target-independent reference meaning, an `ArtifactOracle` defining the exact canonical semantics of the deployed model, and a `ScheduleOracle` proving that the scheduled execution refines that artifact semantics. The approximation relation is between the first two, recorded in `ConformanceEnvelope` / `conformance.json`, not embodied in any single oracle.
* A real staged compiler pipeline: `ArtifactValidationAndUpgrade -> ResolvedCompilePolicy -> QuantGraph -> StaticBudgetReport -> GbInferIR (value/effect IR) -> ObservationPlan -> RangePlan -> StoragePlan -> SramPagePlan -> RomWindowPlan -> ArenaPlan -> GbSchedIR -> ResourceStateValidation -> (AsmIR -> ReachabilityValidation -> PlacedRom -> EncodedRom) -> BuildReports`, with a bounded `FeasibilityRefinementLoop` wrapping `RangePlan` through `GbSchedIR`, bracketed as policy/feasibility/transform/reporting envelopes and sped up by `gbf-store`'s two-level always-on content-addressed `StageCache` (shard-local + global).
* A Bank0 cooperative runtime with deadline-assisted yielding that owns UI, interrupts, scheduling, resumption, a `BankLease`/`BankGuard` ABI, a `video_commit` queue driving UI budget, a versioned SRAM persistence protocol with explicit `DurabilityClass`-stratified record kinds, page states, `PersistChecksum` options, semantic/resume/build identity separation, and atomic `CommitGroupId`-based commit (with HRAM caching but SRAM-authoritative recovery), `ResidencyEpoch`-aware schedule-pack-driven runtime-mode switching (including a `Safe` mode with explicit `SafeModeTrigger`s), multi-resource `FrameBudget`, a `RuntimeDriftMonitor` with `DriftTrigger`s for automatic adaptation, a `FaultPolicy` table with `FaultDomain`s and graded `RecoveryAction`s, and an explicit liveness contract.

The three shared contracts:

* `gbf-hw` owns the **target contract** — actual machine/cartridge profile, physical constraints, and calibration schema.
* `gbf-artifact` owns the **durable model contract** only — deployed artifact lineage (with per-component lowering shards) plus an optional sibling `ReferenceModelBundle` (with frozen `ReferenceProgram` and exported `SemanticCheckpointSchema`) for denotational truth, with content-addressed hashing and blob-backed tensor storage. Faster-moving schemas live in adjacent `gbf-policy`, `gbf-workload`, and `gbf-report`.
* `gbf-abi` owns the **live execution contract** — `InferenceState` (with liveness counters), harness blocks (with a real control-plane `HarnessOp` set), fault codes (including liveness and drift faults), `FaultSnapshot` (with `FaultDomain` and `RecoveryAction`), `FaultPolicy`, trace events, `InterruptPolicy`, `ResourceLease`, semantic checkpoint IDs, `CompatibilityEnvelope`, `BootValidationPlan`, and `BuildIdentityBlock`, with `#[repr(C)]` layouts and a pinned `AbiVersion`.

The three semantic strata (cross-cutting the products, each owning a distinct notion of truth):

* **Denotation** — target-independent reference meaning and quality baseline of the source reference model (`ReferenceModelBundle`); `DenotationalOracle` owns it.
* **Artifact semantics** — exact canonical semantics of the frozen deployed model; `ArtifactOracle` owns it.
* **Operational schedule** — resumable bank-aware execution of artifact semantics on the target; `ScheduleOracle` and the runtime own it.

The four object-level layers (every runtime entity is unambiguously one of these):

* `ArtifactCore` — immutable semantic identity.
* `TargetDataLoweringArtifact` — deterministic derived data form.
* `CompileRequest` — how to build it today (target/profile/objective/calibration/constraints).
* `CompiledBuild` — a versioned `BuildManifest` plus schedule packs, reports, certificates, ROM, listings, maps, and stage-cache refs, with `StabilityTier` annotations.

The closure:

* a measured calibration loop via `gbf-bench`: versioned `WorkloadManifest`s typed by `WorkloadClass`, layered calibration bundle production, measured kernel profiles, cycle-model drift reporting, constrained autotune over a small explicit knob set, and explicit `CompileObjective`s so "faster" is a real optimization target instead of folklore.

Plus the non-negotiable architectural invariants: placement profiles (`StrictOnePerBank` / `Budgeted` / `PackedExperts`) instead of a rigid "one expert equals one bank" law; value/effect `GbInferIR` with explicit effect edges and no buffer commitment; `ObservationPlan` as the stable observation contract with explicit `ObservabilityMode` (invariant vs flexible), `TraceBudget`, and `MetricProbe`s for always-on lightweight telemetry; `StoragePlan` as the bridge between value semantics and spatial scheduling, with `LifetimeClass` replacing the old `survives_yield` boolean; generated yielding as a compiler-visible coroutine ABI with deadline-assisted safe points (`YieldCheckClass`), bounded interrupt latency, and bounded liveness; `ResidencyEpoch` as the intermediate object between `RomWindowPlan` and the final slice schedule; ISR code/data Bank0-resident and bank-agnostic *proven* by `ReachabilityValidation`; the `BankLease`/`BankGuard` ABI as the only legal path to MBC writes; the `video_commit` queue driving UI budget; `gbf-foundation` as the tiny cross-cutting crate for ids, hashes, and semver wrappers; `gbf-kernel` as a dedicated shared crate for kernel families, calling conventions, and autotune dimensions; `gbf-verify` as the owner of independent slow reference implementations (not `gbf-test`); risk-aware `CompileObjective`s with quantile and confidence gates via `RiskPolicy`; `ResolvedCompilePolicy` as the single answer to "what policy governed this build"; `FeasibilityRefinementLoop` as a bounded monotone repair loop with explicit `RepairPolicy` and `RepairProposal`s; profile-guided co-design via load-bearing `HintBundle` (facts/preferences/constraints) and bidirectional `CompilerFeedback`; `CompatibilityEnvelope` and ROM-resident `BuildIdentityBlock` as the single identity handshake for every subsystem; emulator accuracy solved by adapters around SameBoy/BGB rather than re-implementation; a four-profile canonical build set (`Bringup`, `Default`, `Trace`, `Recovery`) with `Speed` and `Size` only once measurements justify them; `DeployabilityEnvelope` + `gbf-train preflight` as the shift-left research-loop filter; independent slow reference implementations for verification-critical algorithms owned by `gbf-verify` so a shared bug cannot become self-validating; `ReferenceNumericProfile` with explicit `DeterminismClass` and `ReductionOrderPolicy` for the denotational stratum so exact equality gates are legal only when the contract admits them; `DataLoweringProfile` decoupling packed-data identity from the full target profile for better cache reuse; `SramPagePlan` as the SRAM analogue of `RomWindowPlan`; `gbf-store` as the dedicated CAS/transport/stage-cache crate so `gbf-artifact` stays schema-only; `RunManifest` and `FailureCapsule` making operational evidence and minimized repros first-class; `ResourceVector` and `FrameBudget` replacing scalar slice budgeting with multi-resource pressure; `CorpusManifest` turning data lineage into a reproducibility peer of build lineage; machine-checkable certificates for critical safety passes consumed by `gbf-verify`; and an always-on content-addressed `StageCache` so layout/backend bugs become ordinary bisect/debug problems.

The single most important architectural correction was the addition of `RomWindowPlan`. The single most important *additional* correction was the three-stratum semantic framing (denotation / artifact semantics / operational schedule) overlaid on the five-product decomposition. The previous pass added three structural corrections: (a) separate **semantic identity** from **build identity** via `CompileRequest`; (b) split **deterministic data lowerings** from **execution/build planning**; and (c) add a formal **observation plan** and **whole-program reachability validation**. The next revision pass added ten more corrections making the architecture a more complete artifact system: (1) split denotational truth from deployed artifact truth via `ReferenceModelBundle`; (2) factor kernels into a dedicated `gbf-kernel` crate; (3) replace `survives_yield: bool` with `LifetimeClass` and add atomic `CommitGroupId`-based persistence; (4) let one `CompiledBuild` contain a `SchedulePack` of multiple runtime-mode variants; (5) make compile objectives quantile-aware and confidence-aware via `RiskPolicy`; (6) add `CompatibilityEnvelope` and ROM-resident `BuildIdentityBlock` as the single identity handshake; (7) make observability perturbation first-class via `ObservabilityMode`, `TraceBudget`, and `FaultSnapshot`; (8) turn harness mode into a real control plane with `StepSlice`, `RunUntilCheckpoint`, `DumpArena`, `InjectFault`, and `PowerCut` ops; (9) emit machine-checkable certificates for critical safety passes consumed by `gbf-verify`; (10) back artifact storage with a content-addressed `blobs/sha256/` store and lazy loading via `BlobRef`. The current revision pass adds eleven more corrections that sharpen boundaries and operational realism: (1) add a bounded `FeasibilityRefinementLoop` with `RepairPolicy`/`RepairProposal`; (2) split calibration into layered `PlatformCalibrationBundle`/`KernelCalibrationBundle`/`RuntimeCalibrationBundle` via `CalibrationSetRef`; (3) introduce `ResolvedCompilePolicy` with `PolicyProvenance`; (4) narrow `ArtifactCore` by moving interaction/decode policy out of immutable identity (`LexicalSpec` + `DecodeCapabilitySet` in core, `InteractionBundle` + `SessionProfile` in aux/session); (5) replace rigid slice budgeting with hybrid timer-assisted safe points (`YieldCheckClass`, `soft_deadline_margin`); (6) simplify persistence by `DurabilityClass` with `PersistChecksum` options and SRAM-authoritative recovery; (7) add first-class `ResidencyEpoch` between `RomWindowPlan` and the schedule; (8) refactor dependency surface with `gbf-foundation` and `gbf-ir-schema`; (9) make `CompiledBuild` concrete with `BuildManifest` and `StabilityTier`; (10) add low-overhead `MetricProbe`s alongside event traces; (11) make the reference side truly reference-grade with `ReferenceNumericProfile` and move slow reference implementations into `gbf-verify`. The latest revision pass adds nine more corrections that sharpen honesty and operational realism: (1) make determinism an explicit contract via `DeterminismClass` and `ReductionOrderPolicy` so exact equality gates are only legal when the contract admits them; (2) decouple data lowering from the full target profile via `DataLoweringProfile`/`DataLoweringProfileId` for better cache reuse and sharper identity boundaries; (3) separate semantic state compatibility from build identity in persistence (`semantic_state_hash`/`resume_abi_hash`/`build_identity_hash` per record kind, with `Continuation` as its own `PersistKind`); (4) add an explicit `SramPagePlan` stage so SRAM page working sets and page-switch behavior have the same first-class planning as ROM visibility; (5) introduce a dedicated `gbf-store` crate for CAS resolution, stage-cache implementation, archive/directory transport, and integrity verification so `gbf-artifact` stays schema-only and `gbf-codegen` does not grow its own blob store; (6) make operational evidence first-class with hierarchical `ConformanceEnvelope`, `RunManifest`, and `FailureCapsule` so runs and failures are queryable scientific records; (7) replace scalar slice budgeting with a multi-resource pressure model (`ResourceVector` per slice, `FrameBudget` for the scheduler); (8) strengthen `gbf-data` into a real corpus-governance layer with `CorpusManifest`, source provenance, dedup policy, and contamination checking; (9) add an explicit `Safe` runtime mode and `Recovery` build profile with `SafeModeTrigger`s and `fallback_runtime_mode` in `RiskPolicy`. The latest revision pass adds ten more corrections focused on long-term evolution, operational state, and user-visible guarantees: (1) make contract evolution a first-class subsystem via `gbf-migrate` with `CompatibilityEpochs`, `MigrationReport`, and host-side upgrade DAGs; (2) make the stage cache shard-aware via `ComponentDigestSet` and `BuildShardManifest` for localized invalidation without changing semantic identity; (3) make ROM/SRAM/overlay/interrupt state explicit in the schedule via `ResourceLease`, `ResourceLeaseKind`, and `ResourceStateValidation`; (4) propagate uncertainty and evidence through the cost model via `CostEstimate`, `EvidenceClass`, and `EstimatedCostDelta`; (5) add service-level objectives for interactive behavior via `ServiceLevelObjective` inside `CompileObjective`; (6) add a runtime drift monitor with automatic demotion via `RuntimeDriftMonitor`, `DriftTrigger`, and `DriftAction`; (7) add fault domains and graded recovery actions via `FaultDomain`, `FaultPolicy`, `RecoveryAction`, and `BootValidationPlan`; (8) turn invariant observability from a promise into a certificate via `ObservabilityCertificate` and `PerturbationSummary`; (9) add `Quality` and `Interactive` workload classes with `ExperienceGate` and lightweight `DecodeTransformSet`; (10) add minimal `ReproducibilityManifest` for scientific reproducibility. The latest revision pass adds eleven more corrections focused on contract narrowness, denotational honesty, and planning completeness: (1) narrow `gbf-artifact` to durable model lineage only, with `gbf-policy`/`gbf-workload`/`gbf-report` as adjacent operational schema families; (2) freeze denotational truth as a `ReferenceProgram` inside `ReferenceModelBundle`; (3) promote `SemanticCheckpointSchema` to an exported contract in `ArtifactAux`; (4) shard target-data lowerings by component via `LoweringShardRef`/`LoweringShard`; (5) remove target-measured kernel timings from `ExportFacts` and add explicit `EvidenceScope` to facts and feedback; (6) add an explicit `OverlayPlan` pass between `RomWindowPlan` and `ArenaPlan`; (7) move `CompilerFeedback` out of `ArtifactAux` into build/run artifacts with explicit promotion; (8) add `ScheduleCostAnalysis` as the single objective-facing cost envelope producer; (9) add `MachineEffect` and `PrivilegeClass` typing to `gbf-asm` so `ReachabilityValidation` validates against explicit intent; (10) simplify `ReproducibilityManifest` and defer `StudyManifest` to a `study_tag`; (11) add best-effort failure reduction / testcase minimization to `FailureCapsule` and `gbf-test`. The latest revision pass adds nine training-contract corrections that make the training side implementable rather than hand-wavy: (1) reframe Burn as training front-end (backend portability, autodiff, optimizers, checkpoints, metrics), not the owner of deployed quantization semantics — `gbf-model` owns ternary projection, activation fake-quant, norm approximation, and export visitation; (2) add `RuntimeChromeBudget` / `RomBudgetSlot` / `BudgetSlotClass` as a build-specific post-chrome bank capacity contract so the trainer reasons about free slot capacities after the shell build; (3) make ternary numeric semantics explicit via `TernaryWeightPlan` / `ScaleGranularity` / `ScaleFormat` / `ThresholdPlan` because `WeightEncoding::Ternary2` alone does not specify quality, accumulator ranges, export bytes, or runtime cost; (4) freeze a dense teacher before hard ternarization, export it as `ReferenceModelBundle`, and train in explicit `TrainPhaseSpec` phases (`DenseTeacherWarmup` → `RouterWarmup` → `ExpertTernaryQat` → `FullNumericQat` → `HardenAndSelect`) with per-phase `QuantHardness` and `RouterTrainMode`; (5) fix the loss to be honest about what is differentiable — add `λ_distill`, `λ_balance`, `λ_zrouter`, `λ_switch` terms, disable `λ_shape`/`λ_overflow` for fixed-shape `Ternary2` experts; (6) restrict MoE to FFN path of selected blocks with two-matrix experts, tied embeddings, optional shared dense FFN branch, and no default GLU; (7) make the router switch-aware with `TemporalSwitchDigest`, `ClipSaturationDigest`, `ExpertPayloadDigest` in `ExportFacts`, `ExpertSlotAffinity` in `CompilePreferences`, z-loss, expert dropout, and temporal smoothness regularization; (8) add shadow export/compile during training via `ShadowCompilePolicy` and Pareto frontier checkpoint selection via `CheckpointFrontierPoint`; (9) make adaptive bank-aware expert structure an explicit M6 research mode via `ExpertShapePolicy::StructuredWidthGates`. The liveness contract, the `video_commit` queue, the `DeployabilityEnvelope` preflight, the always-on `StageCache`, the typed `WorkloadManifest`, and the split `HintBundle` (facts/preferences/constraints) remain load-bearing from previous passes.

That gives you a project that behaves like a maintainable hardware-aware compiler plus cooperative runtime, rather than a heroic pile of emitted bytes. The next useful artifacts are a literal workspace skeleton plus the first shared types: `ArtifactCore`, `ArtifactManifest`, `ArtifactSemanticPayload`, `TargetDataLoweringArtifact`, `ArtifactAux`, `ReferenceModelBundle`, `ReferenceLink`, `ReferenceNumericProfile`, `ReferenceObservationCache`, `ConformanceEnvelope`, `BlobRef`, `BlobCodec`, `CompileRequest`, `ResolvedCompilePolicy`, `PolicyProvenance`, `CompileObjective`, `RiskPolicy`, `RuntimeMode`, `CompileProfileId`, `CalibrationSetRef`, `PlatformCalibrationBundle`, `KernelCalibrationBundle`, `RuntimeCalibrationBundle`, `LexicalSpec`, `InteractionBundle`, `TranscriptSpec`, `SessionProfile`, `DecodeMode`, `DecodeCapabilitySet`, `DecodePolicy`, `HintBundle` (`ExportFacts` / `CompilePreferences` / `BuildConstraints`), `RepairPolicy`, `RepairProposal`, `DataLoweringProfile`, `DataLoweringProfileId`, `DeterminismClass`, `ReductionOrderPolicy`, `SidecarRef`, `WorkloadManifest` / `WorkloadClass` / `AcceptanceMatrix` / `ExecutionMatrix` / `ObservationPolicy`, `ConformanceEnvelope` (hierarchical), `RunManifest` (with `study_tag` and `ReproducibilityManifest`), `FailureCapsule` (with `minimized_workload` and `reduction_log`), `SemanticStratum`, `ReproducibilityManifest`, `CorpusManifest`, `DeployabilityEnvelope`, `SequenceSemanticsSpec`, `TargetProfile`, `KernelResidency`, `KernelSpec` / `KernelSpecId`, `StorageClass`, `LifetimeClass`, `Materialization`, `StorageBinding`, `SramPagePlan`, `GbNode` / `InferOp` (value/effect form), `ObservationPlan` / `SemanticObservation` / `OperationalProbe` / `MetricProbe`, `ObservabilityMode`, `TraceBudget`, `ResourceVector`, `FrameBudget`, `SchedSlice`, `ResidencyEpoch`, `ResourceLease` / `ResourceLeaseKind`, `SchedulePack` / `ModeSwitchPolicy` (with `SafeModeTrigger`s and `DriftTrigger`s), `RuntimeDriftMonitor`, `SchedulerPolicy` (with `FrameBudget`), `YieldCheckClass`, `UiCommitPlan`, `InterruptPolicy`, `SectionRole`, `InferenceState` (with `progress_epoch` / `last_checkpoint` / `no_progress_frames`), `CompatibilityEnvelope`, `BuildIdentityBlock`, `FaultSnapshot`, `HarnessCommandBlock`, `HarnessResultBlock`, `HarnessOp`, `HarnessResultKind`, `FaultCode`, `FaultDomain`, `FaultPolicy`, `RecoveryAction`, `BootValidationPlan`, `CompiledBuild`, `BuildManifest`, `StabilityTier`, `PersistHeader` (with `semantic_state_hash` / `resume_abi_hash` / `build_identity_hash`), `PersistGroupCommit`, `CommitGroupId`, `DurabilityClass`, `PersistChecksum`, `PersistKind` (including `Continuation`), `PageState`, `ServiceLevelObjective`, `ExperienceGate`, `DecodeTransformSet`, `CostEstimate` / `EvidenceClass` / `EstimatedCostDelta`, `ComponentDigestSet`, `BuildShardManifest`, `CompatibilityEpochs`, `MigrationReport`, `ObservabilityCertificate`, `ReferenceProgram`, `SemanticCheckpointSchema`, `LoweringShardRef` / `LoweringShard`, `EvidenceScope`, `OverlayPlan`, `ScheduleCostReport`, `MachineEffect` / `PrivilegeClass`, `SemanticCheckpointId` / `CompactCheckpointId`, `RuntimeChromeBudget` / `RomBudgetSlot` / `BudgetSlotClass`, `TernaryWeightPlan` / `ScaleGranularity` / `ScaleFormat` / `ThresholdPlan`, `TrainPhaseKind` / `TrainPhaseSpec` / `QuantHardness` / `RouterTrainMode`, `ShadowCompilePolicy` / `CheckpointFrontierPoint`, `ExpertShapePolicy`, `TemporalSwitchDigest` / `ClipSaturationDigest` / `ExpertPayloadDigest` / `ExpertSlotAffinity`, and `ExpertTransitionDigest`.

[1]: https://docs.rs/burn "burn - Rust"
[2]: https://gbdev.io/pandocs/MBC5.html "MBC5 - Pan Docs"
[3]: https://gbdev.io/pandocs/Interrupt_Sources.html "Interrupt Sources - Pan Docs"
[4]: https://gbdev.io/pandocs/halt.html "HALT - Pan Docs"
[5]: https://gbdev.io/pandocs/Rendering.html "Rendering - Pan Docs"
[6]: https://github.com/LIJI32/SameBoy "SameBoy - GitHub"
