# RFC F-B1: Compute Bringup (M0.5)

| Field          | Value |
|----------------|-------|
| Author         | bkase / canonicalized by design pass |
| Status         | Draft |
| Feature bead   | To be minted: F-B1 Compute Bringup |
| Open tasks     | T-B1.1 L0 in-WRAM smoke; T-B1.2 L1 request-to-ASM pipeline; T-B1.3 L2 cross-bank operands; T-B1.4 L3 full tiling and streamed output; T-B1.5 L4 cooperative 60 Hz gate; T-B1.6 L5 realism report |
| Closed tasks   | None |
| Plan reference | `history/planv0.md` M0/M1 boundary and engineering rules |
| Glossary       | `history/glossary.md` (banking, lease, residency, slice, yield, liveness, denotational stratum, artifact stratum, operational stratum) |
| Constitution   | §I correctness by construction; §III shifting left; §IV.3 reproducible builds; §V observability; §VI single source of truth |

## 0. TL;DR

F-B1 is the **M0.5 compute bringup** milestone between the M0 foundation stack and M1’s first model-facing pipeline.

M0 gives us the infrastructure: typed assembly, target contracts, ABI layouts, the Bank0 runtime skeleton, harness control plane, emulator adapter, and BankLease/BankGuard banking ABI. M1 will add the first quantized dense kernel, real artifact/oracle/conformance surfaces, and the first durable `CompileRequest`.

F-B1 deliberately does **not** bring up a baby neural network. Instead, it brings up one hard, non-NN, operationally honest workload:

> tiled `i8 × i8 → i32` dense square matrix multiply, with A and B stored in distinct switchable ROM banks, executed by the real cooperative Bank0 runtime, yielding often enough that a VBlank-driven UI widget updates every frame, and verified byte-for-byte against an independent host Rust reference in `gbf-verify`.

The headline run is `N = 128`, with a sweep over `N ∈ {32, 64, 96, 128}` for the final report.

F-B1 closes only when:

1. The ROM executes the matmul through the real runtime/harness/emulator path.
2. A and B are read through distinct ROM banks using BankLease/BankGuard only.
3. The output is streamed tile-by-tile through the existing F-A3 harness dump path.
4. The full output exactly matches `gbf-verify::matmul_reference_i8`.
5. The VBlank frame-service widget updates once per gated frame with `frame_service_misses == 0`.
6. A deterministic `realism_report.v1.json` baseline is checked in under `docs/review/f-b1/artifacts/`.

Normal closure requires the headline `N = 128` run to satisfy all gates above. If the `N = 128` frame gate cannot be met, F-B1 does **not** silently close under the same acceptance criteria. The owner must either:

1. keep the bead open and iterate on in-scope knobs; or
2. land an explicit RFC amendment that changes the closure target and records the measured ceiling as the deliverable.

The waiver path in §9.4 is therefore an escalation path, not an ordinary closure mode.

The main correction from the seed RFC is that F-B1 **does not add a new F-A3 harness opcode**. It uses the existing F-A3 `HarnessOp::DumpArena`/arena-dump control-plane shape. The F-B1 behavior may be described as “dump this output region,” but it does not need a new ABI discriminant.

## 1. Project context — where F-B1 sits in the milestone sequence

### 1.1 What M0 ships (already done or in flight)

Per `planv0.md` M0 deliverables:

- `gbf-asm` — typed LR35902 eDSL, encoder, symbol map, ROM builder. **F-A1**, merged.
- `gbf-hw` — DMG/MBC5 target profiles + calibration schema. **F-A2**, merged.
- `gbf-abi` — live execution contract types (versions, identity, harness, faults, interrupts, leases, checkpoints, traces). **F-A3**, merged.
- `gbf-runtime::banking` — `BankLease`/`BankGuard` ABI. **F-A4**, RFC drafted, in flight.
- Bank0 runtime skeleton — VBlank, keyboard, text output, video commit queue, cooperative scheduler. Scoped under F-A5+.
- `gbf-emu` gameroy adapter — deterministic execution, breakpoints, watchpoints. F-A2 territory; crate skeleton exists.
- `gbf-debug` session file format and rquickjs-scripted agent CLI. F-H3.

The visible state at the start of F-B1: every M0 surface contract type exists in tree, every relevant crate has at least a module-stub `lib.rs`, and F-A1/A2/A3 have shipped.

F-A4 closes before F-B1’s L2 layer can begin (see §8.3).

F-B1 also requires a minimal Bank0 runtime nucleus with:

* VBlank interrupt dispatch;
* cooperative scheduler entry/return;
* legal video-commit path;
* existing F-A3 harness dump support;
* deterministic emulator stepping and breakpoints.

If those pieces are not closed by prior beads, F-B1 owns the minimal implementation needed for this milestone. F-A4 is the single external dependency after the already merged F-A1/F-A2/F-A3 surfaces; it is not the only runtime prerequisite.

### 1.2 What M1 commits to next, after F-B1

Per `planv0.md` M1 deliverables:

> M1: `DenotationalOracle` + `ArtifactOracle` plus a single quantized dense kernel; conformance checking between reference observations and the frozen artifact (first `conformance.json`); first `CompileRequest` wiring.

That sentence packs five distinct architectural commitments into one milestone:

1. The denotational stratum (a frozen `ReferenceModelBundle`, the `ReferenceProgram`, and `DenotationalOracle` consuming them).
2. The artifact stratum (`ArtifactCore` + `ArtifactManifest` + `ArtifactSemanticPayload` + `ArtifactOracle`).
3. The first quantised dense kernel (real `QuantSpec`, real `TernaryWeightPlan` *or* an honest dense-int kernel as a stepping stone, real activation fake-quant, real norm approximation).
4. The first `conformance.json` against a hierarchical `ConformanceEnvelope`.
5. The first real `CompileRequest` wiring — meaning the policy/feasibility/transform/reporting bracketing, `ResolvedCompilePolicy`, `PolicyProvenance`, calibration set refs.

Without F-B1, M1 would simultaneously bring up *all five strata* against an *unproven runtime path*. That is a recipe for a milestone where every failure has too many candidate causes. F-B1’s job is to retire the operational risk behind (3) and (5) in a non-quantised, non-oracle form, leaving M1 free to focus on adding the artifact stratum, the denotational stratum, the quantisation, and the conformance envelope.

### 1.3 What F-B1 retires for M1

By the time M1 begins:

- A request-shaped bringup path, `ComputeBringupRequest → GbInferIR → ...skeletal plans... → AsmIR → ROM → emu → harness`, is *known to work*. M1 replaces this with the real durable `CompileRequest`.
- The cooperative scheduler is *known to host* sustained compute at 60 Hz. M1’s quantised kernel inherits a working scheduling story.
- F-A4’s BankLease ABI is *known to survive* a real cross-bank tile loop. M1’s quantised kernel inherits a working banking story.
- The `realism_report.v1.json` numbers tell M1 *exactly* how many cycles a product-accumulate step costs, how much ROM bandwidth is available, and how many tiles fit in a frame budget. M1’s `QuantSpec` choice is data-driven.
- The host-Rust reference pattern (and its placement in `gbf-verify`) is *known to work* as a witness mechanism. M1 promotes it to oracle witness.
- The harness arena/dump op and the per-frame event trace are *in tree*. M1 uses both unchanged.

F-B1’s job is to retire the operational risk behind the M1 quantised kernel and the M1 `CompileRequest` — not the M1 commitments themselves. It proves that a dense integer loop and request-shaped lowering path can survive the runtime, banking, ROM, emulator, and harness stack. M1 still owns the real quantised dense kernel and the real `CompileRequest`.

### 1.4 Why this is a Feature, not an Epic

F-B1 is one coherent shippable infra-stress milestone with one realism-report deliverable. Epics imply "multiple shippable features under one banner" — that fits *all of M1*, not this. The pattern is the same as F-A1: one Feature, multiple task beads with a strict dep chain, one PR.

## 2. Load-bearing decisions

### 2.1 Workload: dense matmul, not a baby NN

F-B1 uses a single operation:

```text
C = A × B
A: i8[N, N]
B: i8[N, N]
C: i32[N, N]
```

The reference and runtime both use row-major storage and deterministic reduction order:

```text
for i in 0..N:
  for j in 0..N:
    acc = 0_i32
    for k in 0..N:
      acc += i32(A[i, k]) * i32(B[k, j])
    C[i, j] = acc
```

No quantization, no rescale, no activation, no tolerance, no oracle, no artifact stratum.

A baby NN would prematurely pull in artifact semantics, denotational semantics, quantization policy, checkpointed observations, decode policy, and conformance envelopes. F-B1’s job is narrower: prove that the operational substrate can host sustained compute before M1 assigns model meaning to it.

### 2.2 Numeric semantics: `i8 × i8 → i32`

Inputs are signed 8-bit integers. Accumulators and output are signed 32-bit integers.

Worst case for `N = 128`:

```text
max |sum_k a_k b_k| ≤ 128 × 128 × 128 = 2,097,152
```

This fits in `i32` with large margin and does not fit safely in `i16`.

No saturation is allowed. Saturation would add branches and hide errors. Since exact integer matmul has no rounding, the conformance gate is byte-exact.

### 2.3 Matrix sizes

The implementation supports only square sizes that are multiples of 16 and at most 128.

Layer gates use:

| Layer | Size                    |
| ----- | ----------------------- |
| L0    | `N = 16`                |
| L1    | `N = 16`                |
| L2    | `N = 64`                |
| L3    | `N = 128`               |
| L4    | `N = 128`               |
| L5    | `N ∈ {32, 64, 96, 128}` |

`N = 128` is the headline because each operand is exactly 16 KiB:

```text
128 × 128 × 1 byte = 16,384 bytes
```

That fits exactly in one switchable ROM bank and exercises the ROM-window boundary without yet requiring multi-bank sharding for one operand.

Because the operand exactly fills a standard 16 KiB switchable ROM bank, the headline layout requires `offset = 0` for both A and B on the default DMG/MBC5 profile.

### 2.4 Storage map

For the headline `N = 128` run:

| Datum                 | Location                        | Size                          |
| --------------------- | ------------------------------- | ----------------------------- |
| Matrix A              | ROM bank 1, switchable window   | 16,384 bytes                  |
| Matrix B              | ROM bank 2, switchable window   | 16,384 bytes                  |
| Quarter-square table  | Bank0 ROM, permanent residency  | 1,024 bytes                   |
| Kernel code           | Bank0 ROM                       | implementation-sized          |
| Accumulator tile      | WRAM scratch                    | `16 × 16 × i32 = 1,024` bytes |
| A panel               | WRAM scratch                    | `16 × 16 × i8 = 256` bytes    |
| B panel               | WRAM scratch                    | `16 × 16 × i8 = 256` bytes    |
| Output                | streamed through harness        | no full on-cart output buffer |
| Liveness counters     | F-A3 `LivenessCounters` in WRAM | 12 bytes                      |
| Bank shadow           | F-A4 HRAM shadow                | F-A4-defined                  |
| Frame-service widget state | HRAM/WRAM runtime state    | small fixed state             |

A full `128 × 128 × i32` output is 65,536 bytes. F-B1 does not store it all on cartridge. Each completed output tile is exposed through the harness and reassembled on the host.

### 2.5 Tile shape

The canonical output tile is:

```text
m_tile = 16
n_tile = 16
k_tile = 16
```

One output tile has 256 accumulators and is updated by eight K-tiles when `N = 128`. With `N = 128` and `tile = 16`, the tile schedule is exactly `(N/16)^2 = 64` output tiles, `N/16 = 8` K-tiles per output tile, i.e. exactly 512 tile-iterations per matmul — clean integers for the realism report.

The tile size is fixed for F-B1. L4’s 60 Hz gate is achieved by inserting yield checks **inside** the tile loop, not by changing the tile shape.

### 2.6 Yield quantum

The seed RFC proposed yielding at K-tile boundaries. That is too coarse.

A single `16 × 16 × 16` K-tile contains:

```text
16 × 16 × 16 = 4,096 products
```

On a Game Boy, that is far too much work to assume can fit safely before the next VBlank.

F-B1 therefore separates:

```text
storage tile: 16 × 16 × 16
yield quantum: bounded subset of the tile
```

The default L4 yield quantum is:

```text
one k-lane over one output tile = 16 × 16 = 256 products
```

If measured latency requires a smaller quantum, the allowed fallback is:

```text
one k-lane over 4 output rows = 4 × 16 = 64 products
```

If measured latency still cannot meet the frame-service gate, the final in-scope fallback is:

```text
one k-lane over 1 output row = 1 × 16 = 16 products
```

The chosen quantum is recorded in `realism_report.v1.json`.

The tile shape remains `16 × 16 × 16`; only the scheduler-visible compute quantum varies.

### 2.7 Multiply implementation: quarter-square table

F-B1 uses the integer quarter-square identity:

```text
a × b = Q[a + b] - Q[a - b]
Q[x] = floor(x² / 4)
```

For `a, b ∈ [-128, 127]`:

```text
a + b ∈ [-256, 254]
a - b ∈ [-255, 255]
```

So the table covers the full range:

```text
x ∈ [-256, 255]
512 entries
i16 per entry
512 × 2 = 1,024 bytes
```

Maximum value is at `x = -256`: `256² / 4 = 16,384`. All values are non-negative and fit in `i16` with margin.

The quarter-square approach was chosen against three alternatives:

- **Naive shift-and-add.** Variable cycle cost unless deliberately padded, weaker lookup-table pattern reuse for M1, and likely worse scheduler predictability. F-B1 should not claim a precise speedup before measurement.
- **Booth multiplication or other CPU-intensive variants.** All expected to be worse than quarter-square table on this CPU; not entertained further.
- **HRAM-mirrored sub-table for hot entries.** Tempting on the inner loop but contends with F-A4's HRAM bank-shadow and the widget state; deferred to M1 if measurements suggest it pays off.

Estimated logical work per product-accumulate: two table lookups, one `i16` subtract, sign-extension to `i32`, and one `i32` accumulator add. The actual LR35902 cost is measurement-defined because address formation, table indexing, carry propagation, loop overhead, and memory traffic dominate the naive formula.

The quarter-square implementation is chosen because it is deterministic, branch-light, and establishes the lookup-table idiom that M1 will reuse for activations, softmax, and quantization rescales. The realism report, not this RFC text, decides whether it is actually faster than alternatives.

The production table generator lives in `gbf-codegen`. An independent reference generator lives in `gbf-verify`. A test asserts byte equality between the two generated tables.

The table is Bank0-resident. F-B1 does not mirror it into HRAM. HRAM is reserved for the runtime’s banking shadow, scheduler-critical state, and small UI state.

### 2.8 Kernel residency

The matmul kernel code lives in Bank0.

This is required because the kernel performs bank switching. Code executing from the switchable ROM window cannot safely switch that same window out from underneath itself. Keeping the kernel in Bank0 makes the bank visibility story simple:

```text
Bank0: runtime + scheduler + matmul kernel + quarter-square table
ROMX:  one selected operand bank at a time
```

Bank0 is scarce, but F-B1 is a bringup milestone. If the kernel is too large for Bank0, that is a load-bearing discovery for M1. F-B1 should not hide it behind a switchable-bank kernel that makes banking unsound.

### 2.9 Banking path

A and B live in distinct nonzero ROM banks:

```text
a_bank != b_bank
a_bank != 0
b_bank != 0
```

The generated code may not write directly to MBC5 registers. It must request bank visibility through F-A4’s BankLease/BankGuard ABI.

The only code allowed to perform MBC writes is the audited runtime banking implementation.

### 2.10 Output dump path

F-B1 uses the F-A3 harness control plane’s existing memory/arena dump operation.

The protocol is:

1. Runtime completes one accumulator tile in WRAM.
2. Runtime reaches a harness-visible tile-complete safe point, with no active `BankLease`, and stops reuse of the accumulator slot.
3. Host issues the existing dump command for the accumulator tile address and length.
4. Host reads exactly 1,024 bytes.
5. Host appends those bytes into the output matrix in deterministic row-major tile order.
6. Host acknowledges/resumes the ROM.
7. Runtime reuses the same accumulator tile for the next output tile.

The safe point must be explicit: either a harness trap, breakpoint, or existing F-A3 control-plane pause. F-B1 must not rely on host polling a WRAM address while the ROM continues and races reuse of the accumulator tile.

Tile identity is not a new harness ABI. The host derives:

```text
tile_index
mt
nt
```

from the known tile schedule and the count of tile-complete safe points observed for the current run.

The checked-in `matmul_n128.map.json` records:

```text
source_wram_addr
byte_len = 1024
tile_internal_layout = row_major_i32_le
```

An optional WRAM `TileReadyRecord` may be used as a debug assertion, but it must be dumped through the existing F-A3 memory/arena dump operation and must not introduce a new `HarnessOp` discriminant.

No new `HarnessOp` variant is introduced by F-B1.

### 2.11 Verification reference

The reference implementation lives in:

```text
gbf-verify::matmul
```

It is pure Rust, deterministic, slow, independent, single-threaded, and does not use SIMD.

Shared shape/newtype definitions used by production code do **not** live in `gbf-verify`. Verification may depend on production-neutral ABI/IR shape types, but production crates must not depend on the verifier.

The shared definitions live in:

```text
gbf-abi::compute_shape
```

or, if the project prefers IR ownership:

```text
gbf-ir::shape
```

`gbf-verify::matmul` consumes those shape types and provides the independent slow reference.

This is the first operational use of the rule:

> Verification-critical algorithms have an independent slow reference implementation in `gbf-verify`.

### 2.12 Report deliverable

F-B1’s durable output is:

```text
docs/review/f-b1/artifacts/realism_report.v1.json
```

The report records the cost of this bringup kernel:

* cycles per product-accumulate
* cycles per output tile
* cycles per full matmul
* ROM bytes read
* bank switches
* lease acquire/release balance
* WRAM/HRAM/SRAM peak use
* frames to completion
* VBlank misses
* widget updates
* interrupt/yield latency
* byte-exact conformance result
* reproducibility metadata

The report is a baseline, not a future regression threshold. M1 may promote selected fields into gates once the first quantized dense kernel exists.

## 3. Goals

F-B1 implements:

1. `gbf-verify::matmul_reference_i8`, including checked matrix view types.
2. A production-neutral `OperandFixtureSpec::DeterministicAffineV1`, with a production operand materializer in `gbf-codegen` and an independent verifier materializer in `gbf-verify`.
3. Independent production and reference quarter-square table generators.
4. A skeletal F-B1-local compile request, named `ComputeBringupRequest`, whose production-used shape, fixture, and layout types do not depend on `gbf-verify`.
5. A skeletal `gbf-ir::infer` node for `MatmulI8`.
6. A skeletal ROM-window plan for fixed Bank0 resources and two switchable operand banks.
7. A skeletal storage plan for WRAM accumulator/panels and harness-streamed output.
8. A skeletal arena plan with one reusable accumulator tile.
9. A Bank0-resident matmul kernel emitted through `gbf-asm`.
10. BankLease/BankGuard acquire/release around operand panel copies.
11. A narrow F-B1 resource/reachability validator:

    * generated matmul code contains no raw MBC writes;
    * runtime banking code is the only MBC-writing path;
    * VBlank/ISR code and data are Bank0-resident;
    * generated compute sections are not ISR-resident;
    * no yield occurs while a bank lease is active;
    * no harness tile-complete pause occurs while a bank lease is active;
    * all F-B1 BankLease records are balanced.
12. A tile dump/reassembly harness using the existing F-A3 dump operation.
13. A VBlank-driven four-hex-digit frame-service widget.
14. Emulator frame-event instrumentation sufficient to prove the 60 Hz gate.
15. `gbf-bench` workload runner for `MatmulI8Bringup`.
16. `gbf-report` schema/types for `realism_report.v1`.
17. A deterministic frozen baseline report and review packet under `docs/review/f-b1/`.

## 4. Non-goals

F-B1 does **not** implement:

* Quantization.
* `QuantSpec`.
* `TernaryWeightPlan`.
* Scale formats.
* Threshold plans.
* Activation fake-quant.
* Norm approximation.
* Any neural-network layer.
* `ReferenceModelBundle`.
* `ArtifactCore`.
* `ArtifactOracle`.
* `DenotationalOracle`.
* `ScheduleOracle`.
* `conformance.json`.
* `ConformanceEnvelope`.
* `ObservationPlan`.
* `RangePlan`.
* `SramPagePlan`.
* `OverlayPlan`.
* Multi-op `GbInferIR`.
* Full M1 `CompileRequest`.
* `ResolvedCompilePolicy`.
* `gbf-policy` integration.
* Autotune.
* Runtime drift monitor.
* Safe runtime mode.
* Fault-policy recovery exercise.
* CGB/GBC support.

F-B1 may introduce skeletal shapes that M1 expands, but it must not pretend those skeletal shapes are the final M1 contracts.

## 5. Anti-goals

Reviewers must refuse these changes:

| Anti-pattern                                 | Reason                                               | Correct answer                  |
| -------------------------------------------- | ---------------------------------------------------- | ------------------------------- |
| Add `quant_policy: Option<_>` to the request | Pulls quantization into M0.5                         | No quant fields                 |
| Stub an oracle for matmul                    | Category error                                       | Use `gbf-verify` reference only |
| Emit empty `conformance.json`                | Teaches wrong data flow                              | M1 emits first conformance file |
| Generalize `MatmulI8` to `Matmul<T>`         | M1 quantized kernels have different effects          | Keep concrete `MatmulI8`        |
| Add `RangePlan` slots                        | Pulls M1 range semantics forward                     | No range plan                   |
| Allow tolerance in output compare            | Exact integer matmul has no rounding                 | Byte-exact only                 |
| Put kernel in switchable ROM                 | Kernel switches ROM banks                            | Kernel is Bank0                 |
| Add a new harness opcode                     | Existing dump path suffices                          | Use F-A3 dump op                |
| Use direct MBC writes in generated code      | Violates BankLease discipline                        | Only runtime banking writes MBC |
| Yield while a BankLease is active            | Lets scheduler/ISR observe ambiguous ROMX state      | Copy panels, release lease, then yield |
| Store full output in SRAM                    | 64 KiB output does not fit standard SRAM assumptions | Stream tile output              |
| Move multiply table to switchable bank       | Compute yields inside table-using regions; ROMX table residency would require holding or reacquiring a lease across scheduler-visible compute | Bank 0, fixed residency         |
| Make L4's gate a soft warning                | Defeats milestone purpose; the gate exists to surface the cost ceiling | Hard gate from day one |
| Bundle the reference into a `ReferenceModelBundle` | Distorts M1's bundle format                    | Plain function in `gbf-verify`  |
| Generate ROM operands by calling `gbf-verify` from production crates | Reverses the verifier dependency direction | Use `OperandFixtureSpec` plus independent production/verifier materializers |
| Add tile-ready metadata by minting a new harness opcode | Reintroduces the F-A3 opcode drift this RFC avoids | Derive tile identity from schedule/trap count or use an existing F-A3 checkpoint/trace surface |

## 6. Pipeline

Canonical F-B1 pipeline:

```text
ComputeBringupRequest
  └─ gbf-codegen::import
       validates target/profile/tile/operand layout
       hashes canonical request

GbInferIR
  └─ one MatmulI8 node

BringupRomWindowPlan
  ├─ Bank0 permanent resources
  │   ├─ runtime nucleus
  │   ├─ matmul kernel
  │   └─ quarter-square table
  └─ switchable operand resources
      ├─ A in bank 1
      └─ B in bank 2

BringupStoragePlan
  ├─ accumulator tile -> WRAM scratch
  ├─ A panel -> WRAM scratch
  ├─ B panel -> WRAM scratch
  └─ output -> harness stream sink

BringupArenaPlan
  └─ one reusable WRAM accumulator tile

gbf-codegen::lower_asm
  └─ Bank0 matmul kernel AsmIR

gbf-asm
  ├─ layout
  ├─ relaxation/legalization
  ├─ encoder
  ├─ ROM builder
  ├─ .sym
  └─ .lst

gbf-runtime
  ├─ Bank0 scheduler
  ├─ BankLease/BankGuard runtime
  ├─ VBlank handler
  ├─ video_commit queue
  └─ frame-service widget

gbf-emu
  ├─ deterministic execution
  ├─ harness control plane
  └─ frame-event trace

host harness
  ├─ tile dump reassembly
  ├─ byte-exact compare against gbf-verify
  └─ metric aggregation

gbf-report
  └─ realism_report.v1.json
```

The diagram is the canonical M1 pipeline shape with everything F-B1 doesn't need *removed*: no `ArtifactCore`, no `ResolvedCompilePolicy`, no `QuantGraph`, no `RangePlan`, no `ObservationPlan`, no `SramPagePlan`, no `OverlayPlan`, no `GbSchedIR`, and no full M1 `ResourceStateValidation` / `ReachabilityValidation` pass.

F-B1 does, however, enforce the narrow bringup validator described in §7.8:

* generated compute sections contain no raw MBC writes;
* generated compute sections are not ISR-reachable;
* VBlank/ISR code and data are Bank0-resident;
* no yield or harness tile-complete pause occurs while a `BankLease` is active;
* MBC writes are confined to the audited runtime banking implementation.

Everything beyond that subset is deferred to M1. Each absent stage is named in §7's per-type "Deferred to M1" lists.

## 7. Core types

### 7.1 Shared compute shape types

```rust
pub struct SquareDim {
    n: u16,
}

impl SquareDim {
    pub fn new(n: u16) -> Result<Self, MatmulShapeError>;
    pub fn n(self) -> u16;
    pub fn elem_count(self) -> usize;
}
```

Validation:

* `n > 0`
* `n % 16 == 0`
* `n <= 128`

Owner:

```text
gbf-abi::compute_shape
```

or:

```text
gbf-ir::shape
```

Production crates may use this type. `gbf-verify` may also use this type. Production crates must **not** depend on `gbf-verify`.

### 7.1.1 `gbf-verify::matmul`

```rust
pub struct MatrixI8<'a> {
    dim: SquareDim,
    data: &'a [i8],
}

pub struct MatrixI32 {
    dim: SquareDim,
    data: Vec<i32>,
}
```

Validation:

* slice length must equal `n × n`
* layout is row-major
* no transposed or strided view in F-B1

Reference function:

```rust
pub fn matmul_reference_i8(
    a: MatrixI8<'_>,
    b: MatrixI8<'_>,
) -> Result<MatrixI32, MatmulReferenceError>;
```

Semantics:

* loops are `i`, `j`, `k`
* accumulator is `i32`
* each operand is sign-extended to `i32` before multiply
* no saturation
* no SIMD
* no parallelism
* output bytes are little-endian `i32`, row-major

Reference fixture materialization:

```rust
pub fn deterministic_matrix_a(dim: SquareDim) -> Vec<i8>;
pub fn deterministic_matrix_b(dim: SquareDim) -> Vec<i8>;
```

The fixture formulas are pure arithmetic, not RNG-backed. These formulas are canonical, not examples:

```text
A[i, j] = (((73*i + 37*j + 19) mod 256) - 128) as i8
B[i, j] = (((29*i + 91*j + 11) mod 256) - 128) as i8
```

Implementation note: compute the formula in at least `i32`/`u32`, subtract 128 in a signed type, then convert to `i8`. Do not rely on accidental unsigned underflow.

Operand ROM serialization is two's-complement `i8 as u8`, row-major.

Production operand materialization:

```text
gbf-codegen::fixtures::materialize_operand_fixture
```

The production materializer consumes `OperandFixtureSpec::DeterministicAffineV1` and emits the A/B ROM byte payloads.

Independence rule:

* production materialization lives outside `gbf-verify`;
* verifier materialization lives in `gbf-verify`;
* production and verifier materializers must not call the same helper;
* a test asserts byte equality of the production and verifier fixture bytes for every F-B1 review-packet size;
* production crates must not depend on `gbf-verify` to build operand ROM payloads.

### 7.2 Quarter-square table

Production owner:

```text
gbf-codegen::tables::quarter_square_table_i16
```

Reference owner:

```text
gbf-verify::matmul::quarter_square_table_reference_i16
```

Canonical table:

```rust
pub const QUARTER_SQUARE_MIN: i16 = -256;
pub const QUARTER_SQUARE_MAX: i16 = 255;
pub const QUARTER_SQUARE_LEN: usize = 512;

pub fn quarter_square_table_i16() -> [i16; 512];
```

Indexing:

```text
idx(x) = x + 256
```

Validation:

* table length is 512
* serialized byte size is 1,024
* every value is non-negative
* max value is 16,384
* production generator equals independent reference generator
* table multiplication matches direct multiplication for every `a,b ∈ i8`

Independence rule:

* the production generator and reference generator must not call the same helper;
* the reference generator uses direct `i32` arithmetic over the canonical range;
* the exhaustive multiplication test computes direct `i32(a) * i32(b)` independently of both table generators.

### 7.3 `ComputeBringupRequest`

F-B1 does not introduce the final M1 `CompileRequest`. It introduces a deliberately named local request:

```rust
pub struct ComputeBringupRequest {
    pub target_profile_id: TargetProfileId,
    pub kernel_impl_id: KernelImplId,
    pub compile_profile_id: CompileProfileId,
    pub matrix_dim: SquareDim,
    pub tile_size: TileSize,
    pub operand_fixture: OperandFixtureSpec,
    pub operand_layout: OperandLayout,
    pub yield_quantum: YieldQuantum,
}
```

Validation:

* `compile_profile_id` must be Bringup.
* `target_profile_id` must identify a supported DMG/MBC5 bringup profile.
* `tile_size` must be `16 × 16 × 16`.
* `matrix_dim` must satisfy `SquareDim`.
* `operand_fixture` must be `DeterministicAffineV1`.
* for closure/review-packet runs, `operand_layout` must put A and B in distinct nonzero ROM banks.
* A and B must fit in their assigned banks.
* `yield_quantum` must be one of the allowed F-B1 variants.

Allowed `kernel_impl_id` values in F-B1:

```text
QuarterSquareTableV1
```

Additional kernel implementations require an RFC amendment or a later feature bead.

```rust
pub enum OperandFixtureSpec {
    DeterministicAffineV1,
}
```

`OperandFixtureSpec` is part of the request hash. The production ROM builder materializes operand bytes from this spec. The verifier independently materializes the same fixture and compares hashes before comparing outputs.

```rust
pub struct TileSize {
    pub m: u16,
    pub n: u16,
    pub k: u16,
}
```

Allowed value in F-B1:

```text
m = 16, n = 16, k = 16
```

```rust
pub enum YieldQuantum {
    KLaneFullTile,      // 16 × 16 = 256 products
    KLaneRows4,         // 4 × 16 = 64 products
    KLaneRow,           // 1 × 16 = 16 products
}
```

Default:

```text
KLaneFullTile
```

If L4 cannot meet the gate with `KLaneFullTile`, `KLaneRows4` is allowed and must be recorded in the report.

If L4 still cannot meet the gate with `KLaneRows4`, `KLaneRow` is allowed and must be recorded in the report. If `KLaneRow` still cannot meet the gate, the bead cannot close under normal F-B1 acceptance.

```rust
pub enum OperandLayout {
    /// Only valid for L0/L1 smoke profiles with `matrix_dim = 16`.
    /// Not valid for closure artifacts or the L5 report.
    WramSmoke,

    DistinctRomBanks {
        a: BankedOperandPlacement,
        b: BankedOperandPlacement,
    },
}
```

`RomBankId` is a checked newtype over `u16`.

```rust
pub struct BankedOperandPlacement {
    pub bank: RomBankId,
    pub offset: RomBankOffset,
}

pub struct RomBankOffset(u16);
```

Validation:

* bank id is valid for the selected target profile;
* for MBC5, bank id fits the supported ROM-bank range;
* A and B banks are nonzero;
* A and B banks are distinct;
* `offset + length <= rom_bank_size`, using checked arithmetic widened to at least `u32`;
* for `N = 128`, operand length is exactly one 16 KiB bank, so operand offset must be zero unless the target profile explicitly provides a larger switchable bank;
* generated ROM layout must not overlap Bank0 interrupt vectors, header, runtime nucleus, kernel, or quarter-square table.

**Deferred to M1** (these fields *will not* appear on `ComputeBringupRequest`; M1 introduces them on the real `CompileRequest`):

* `quant_policy: QuantPolicy`
* `calibration_set_ref: CalibrationSetRef`
* `risk_policy: RiskPolicy`
* `objective: CompileObjective`
* `repair_policy: RepairPolicy`
* `observability_mode: ObservabilityMode`
* `trace_budget: TraceBudget`
* `data_lowering_profile_id: DataLoweringProfileId`

The body of `ComputeBringupRequest` must carry an explicit `// M1: real CompileRequest will replace this type entirely; deferred fields are listed in F-B1 §7.3` comment so a reader can see what is intentionally missing.

### 7.4 `gbf-ir::infer::MatmulI8`

F-B1 adds one IR node:

```rust
pub enum InferNode {
    MatmulI8(MatmulI8Node),
    End,
}

pub struct MatmulI8Node {
    pub a: TensorBinding,
    pub b: TensorBinding,
    pub out: TensorBinding,
    pub dim: SquareDim,
    pub tile_size: TileSize,
}
```

F-B1 invariants:

* graph contains exactly one `MatmulI8` and one `End`
* A and B are read-only
* output is harness-streamed
* no effect-edge graph yet
* no range annotations
* no observation hooks
* no quantization metadata

**Deferred to M1:**

* All other op kinds (`Norm`, `Activation`, `RouterTopK`, `ExpertDispatch`, `Embed`, `Unembed`, ...).
* Effect edges. F-B1's single-op IR has trivial effect ordering; M1 introduces the effect-edge graph.
* `ObservationPlan` visitor hooks.
* `RangePlan` annotation slots.
* `LifetimeClass` on `TensorBinding`.

The type body must contain a clear M1 expansion comment naming deferred fields and stages.

### 7.5 `BringupRomWindowPlan`

```rust
pub struct BringupRomWindowPlan {
    pub kernel: RomResidency,
    pub multiply_table: RomResidency,
    pub operand_a: RomResidency,
    pub operand_b: RomResidency,
}
```

```rust
pub enum RomResidency {
    PermanentBank0 {
        offset: u16,
        length: u16,
    },
    SwitchableRom {
        bank: RomBankId,
        offset: u16,
        length: u16,
    },
}
```

Resolved decision: no numeric `ResidencyEpoch(0)` sentinel. Bank0 permanence is named as `PermanentBank0`.

Validation:

* kernel is `PermanentBank0`
* multiply table is `PermanentBank0`
* A and B are `SwitchableRom`
* A bank and B bank are distinct
* A and B banks are nonzero
* lengths match `N × N`
* multiply table length is 1,024 bytes
* Bank0 permanent resources fit with reserved vectors/header/runtime sections
* report includes `bank0_used_bytes` and `bank0_free_bytes`

**Deferred to M1:**

* Multiple residency epochs and the schedule of switches between them.
* `OverlayPlan` integration.
* Hot/cold ranking of switchable resources.
* Multi-tenant residency arbitration.

### 7.6 `BringupStoragePlan`

```rust
pub struct BringupStoragePlan {
    pub accumulator: StorageBinding,
    pub a_panel: StorageBinding,
    pub b_panel: StorageBinding,
    pub output: OutputBinding,
}
```

```rust
pub enum StorageClass {
    WramScratch,
}

pub struct StorageBinding {
    pub class: StorageClass,
    pub offset: u16,
    pub length: u16,
}

pub enum OutputBinding {
    HarnessTileStream {
        source: WramArenaSlot,
        tile_bytes: u16,
        order: TileOrder,
    },
}

pub enum TileOrder {
    MtMajorNtMinorRowMajorI32Le,
}
```

Canonical sizes:

```text
accumulator.length = 1,024
a_panel.length     = 256
b_panel.length     = 256
output.length      = 1,024 per streamed tile
```

F-B1 does not introduce the full M1 `LifetimeClass`. Instead, each binding is documented as single-tile reusable.

**Deferred to M1:**

* `LifetimeClass` taxonomy (replaces F-B1's ad-hoc single-tile-reusable convention).
* `Materialization` choices.
* Persistence routing (`SramPersistKind`).
* Multi-binding resolution for shared tensors.

### 7.7 `BringupArenaPlan`

```rust
pub struct BringupArenaPlan {
    pub accumulator_tile: WramArenaSlot,
    pub a_panel: WramArenaSlot,
    pub b_panel: WramArenaSlot,
}
```

Validation:

* slots do not overlap
* slots fit in WRAM
* accumulator aligned to 4 bytes
* total WRAM scratch pressure is reported

No allocator is introduced in F-B1.

**Deferred to M1:**

* A real bump allocator.
* Per-`LifetimeClass` arena partitions.
* Peak-pressure analysis.
* `ResourceVector` integration.

### 7.8 Lowering contract

The generated kernel is Bank0-resident and follows this logical structure:

```text
for mt in output_tile_rows:
  for nt in output_tile_cols:
    zero accumulator[16,16]

    for kt in k_tiles:
      lease A bank
      copy A[mt:mt+16, kt:kt+16] -> a_panel
      release A bank

      lease B bank
      copy B[kt:kt+16, nt:nt+16] -> b_panel
      release B bank

      assert no BankLease is active before entering compute/yield region

      for kk in 0..16:
        if yield_quantum == KLaneFullTile:
          for mm in 0..16:
            for nn in 0..16:
              acc[mm,nn] += quarter_square_mul(a_panel[mm,kk], b_panel[kk,nn])
          maybe_yield()

        if yield_quantum == KLaneRows4:
          for mm_base in [0, 4, 8, 12]:
            for mm in mm_base..mm_base+4:
              for nn in 0..16:
                acc[mm,nn] += quarter_square_mul(a_panel[mm,kk], b_panel[kk,nn])
            maybe_yield()

        if yield_quantum == KLaneRow:
          for mm in 0..16:
            for nn in 0..16:
              acc[mm,nn] += quarter_square_mul(a_panel[mm,kk], b_panel[kk,nn])
            maybe_yield()

    expose accumulator tile for harness dump
```

The host reassembles output tiles in this order:

```text
mt major, nt minor
```

Each tile is row-major internally.

`ReachabilityValidation` is *annotated* (each section gets a `SectionRole`, `InterruptSafetyKind`, `PrivilegeClass`).

F-B1 enforces only the narrow subset needed for this milestone:

* generated compute sections contain no raw MBC writes;
* generated compute sections cannot be reached from ISR entrypoints;
* VBlank/ISR code and data are Bank0-resident;
* no yield occurs while a `BankLease` is active;
* MBC writes are confined to the audited runtime banking implementation;
* unknown writes into MBC address ranges from generated sections fail closed.

Full M1 `ReachabilityValidation` is not enforced. F-B1 enforces only the listed bringup subset.

## 8. L0–L5 layer plan

### 8.1 L0 — in-WRAM smoke

Purpose: isolate basic runtime, emulator, harness, encoder, and reference correctness.

| Field         | Value                                      |
| ------------- | ------------------------------------------ |
| Size          | `N = 16`                                   |
| Banking       | none                                       |
| Compiler path | hand-authored or minimal AsmIR; no `ComputeBringupRequest` required |
| UI            | none                                       |
| Output        | one accumulator tile dump                  |
| Gate          | dumped bytes exactly equal reference bytes |

Failure mode it isolates: M0 infra (encoder, emu, runtime nucleus, harness).

Acceptance gates:

```text
cargo test -p gbf-verify -- matmul_reference_i8_known_fixture
cargo test -p gbf-codegen -- quarter_square_table_matches_verify
cargo test -p gbf-codegen -- quarter_square_mul_exhaustive_i8
cargo test -p gbf-emu -- f_b1_l0_wram_matmul_dump_matches_reference
```

### 8.2 L1 — request to IR to AsmIR

Purpose: prove the skeletal compile path works.

| Field         | Value                                                |
| ------------- | ---------------------------------------------------- |
| Size          | `N = 16`                                             |
| Banking       | `WramSmoke` operand layout                           |
| Compiler path | `ComputeBringupRequest → MatmulI8 → AsmIR`           |
| UI            | none                                                 |
| Gate          | same byte-exact result as L0, but emitted by codegen |

Failure mode it isolates: the new IR / codegen path at tiny size. It does not isolate banking from codegen; that isolation is handled by L2.

Acceptance gates:

```text
cargo test -p gbf-codegen -- f_b1_request_validation_accepts_l1_smoke_layout
cargo test -p gbf-codegen -- f_b1_request_validation_rejects_smoke_layout_for_n_gt_16
cargo test -p gbf-ir -- f_b1_matmul_ir_single_node_shape
cargo test -p gbf-codegen -- f_b1_l1_lowering_is_deterministic
cargo test -p gbf-emu -- f_b1_l1_compiled_matmul_matches_reference
```

### 8.3 L2 — cross-bank operands

Purpose: make F-A4 banking real under compute pressure. Hard dependency on F-A4.

| Field  | Value                                                                   |
| ------ | ----------------------------------------------------------------------- |
| Size   | `N = 64`                                                                |
| A      | switchable ROM bank 1                                                   |
| B      | switchable ROM bank 2                                                   |
| Kernel | Bank0                                                                   |
| Table  | Bank0                                                                   |
| UI     | none                                                                    |
| Gate   | byte-exact output; balanced leases; no raw MBC writes in generated code |

Failure mode it isolates: F-A4's BankLease ABI under real cross-bank load.

Acceptance gates:

```text
cargo test -p gbf-codegen -- f_b1_rom_window_plan_distinct_banks
cargo test -p gbf-codegen -- f_b1_l2_lowering_uses_banklease_ops
cargo test -p gbf-codegen -- f_b1_l2_generated_code_has_no_raw_mbc_writes
cargo test -p gbf-runtime -- f_b1_l2_banklease_balance
cargo test -p gbf-runtime -- f_b1_l2_no_yield_while_banklease_active
cargo test -p gbf-emu -- f_b1_l2_cross_bank_matmul_matches_reference
```

### 8.4 L3 — full tiling and streamed output

Purpose: prove the full `N = 128` tiling and tile dump/reassembly path.

| Field   | Value                                          |
| ------- | ---------------------------------------------- |
| Size    | `N = 128`                                      |
| Banking | A and B in distinct ROM banks                  |
| Output  | streamed one 1,024-byte tile at a time         |
| UI      | none                                           |
| Gate    | full host-reassembled output matches reference |

Failure mode it isolates: tile schedule, output spill, bank-switch budget.

Acceptance gates:

```text
cargo test -p gbf-codegen -- f_b1_l3_tile_schedule_covers_output_once
cargo test -p gbf-codegen -- f_b1_l3_arena_slots_do_not_overlap
cargo test -p gbf-emu -- --ignored f_b1_l3_streamed_output_matches_reference_n128
cargo test -p gbf-bench -- --ignored f_b1_l3_records_tile_metrics
```

### 8.5 L4 — cooperative yielding and frame gate

Purpose: prove sustained compute can coexist with the runtime’s frame cadence.

| Field    | Value                                                       |
| -------- | ----------------------------------------------------------- |
| Size     | `N = 128`                                                   |
| UI       | VBlank-driven 4-hex-digit frame-service widget              |
| Yield    | default `KLaneFullTile`, fallback `KLaneRows4`, final fallback `KLaneRow` |
| Liveness | `progress_epoch` advances at least once per compute quantum |
| Gate     | byte-exact output and `frame_service_misses == 0`           |

Failure mode it isolates: cooperative scheduler under sustained compute, video_commit, liveness contract, yield-insertion correctness.

Acceptance gates:

```text
cargo test -p gbf-runtime -- f_b1_frame_widget_tile_render_known_vector
cargo test -p gbf-runtime -- f_b1_frame_widget_ticks_once_per_vblank
cargo test -p gbf-emu -- f_b1_frame_event_trace_orders_vblank_and_widget
cargo test -p gbf-emu -- --ignored f_b1_l4_n128_no_frame_service_misses
cargo test -p gbf-emu -- --ignored f_b1_l4_liveness_no_progress_frames_bounded
cargo test -p gbf-emu -- --ignored f_b1_l4_output_matches_reference
```

Hard gate:

```text
frame_service_misses == 0
widget_update_count == gated_frame_count
scheduler_service_count == gated_frame_count
max_no_progress_frames <= 1
max_unyielded_compute_mcycles <= 17,556 - soft_deadline_margin_mcycles
max_bank_lease_hold_mcycles <= 17,556 - soft_deadline_margin_mcycles
byte_exact_match == true
yield_while_bank_lease_active_count == 0
harness_pause_while_bank_lease_active_count == 0
```

All count equality gates in this block are computed over the gated frame interval only, not over startup, shutdown, or host-paused tile-dump intervals.

### 8.6 L5 — realism report

Purpose: freeze the M0.5 empirical baseline.

| Field           | Value                                      |
| --------------- | ------------------------------------------ |
| Sweep           | `N ∈ {32, 64, 96, 128}`                    |
| Report          | `realism_report.v1.json`                   |
| Reproducibility | two consecutive regenerations diff to zero |
| Gate            | report checked in and validated            |

Failure mode it isolates: instrumentation completeness.

Acceptance gates:

```text
cargo test -p gbf-report -- realism_report_v1_schema_accepts_fixture
cargo test -p gbf-report -- realism_report_v1_rejects_missing_required_fields
cargo test -p gbf-bench -- f_b1_report_aggregation_known_fixture
./scripts/review/f-b1/regen.sh
./scripts/review/f-b1/verify-packet.sh
```

## 9. Frame gate semantics

The project reports scheduler and instruction costs in **M-cycles**.

DMG frame timing:

```text
one frame = 70,224 dots/T-cycles = 17,556 M-cycles
```

The common phrase “60 fps” means “one update per VBlank on DMG,” approximately 59.7 Hz.

### 9.1 Frame event types

F-B1 adds emulator-side frame events:

```rust
pub struct FrameEventEnvelope {
    pub seq: u64,
    pub event: FrameEvent,
}

pub enum FrameEvent {
    VBlankFired {
        frame: u32,
        mcycle_since_boot: u64,
    },
    WidgetTickDispatched {
        frame: u32,
        mcycle_since_boot: u64,
    },
    SchedulerServicedFrame {
        frame: u32,
        mcycle_since_boot: u64,
    },
    YieldReturnedToScheduler {
        frame: u32,
        mcycle_since_boot: u64,
        remaining_frame_mcycles_i32: i32,
        completed_quantum_products: u16,
        compute_progress_epoch: u32,
    },
    ComputeProgressEpochAdvanced {
        frame: u32,
        mcycle_since_boot: u64,
        compute_progress_epoch: u32,
    },
}
```

These are host/emulator report events, not new ROM ABI types.

`seq` gives a deterministic total order for events with the same `mcycle_since_boot`. Frame-order checks use lexicographic ordering over `(mcycle_since_boot, seq)`.

`remaining_frame_mcycles_i32` is signed so late service can be reported as a negative margin instead of underflowing or being clipped to zero.

Cycle attribution rule:

* `cycles_per_product_accumulate` is computed from emulator cycle deltas over known product-count regions.
* Tile dump pause time is excluded from compute-cost fields and reported separately as harness overhead if measured.
* VBlank ISR time is included in end-to-end frame counts, but compute micro-cost fields must state whether ISR cycles were excluded or included.

### 9.2 Missed frame definition

A gated frame `F` is missed if any of the following is true:

1. no `WidgetTickDispatched { frame: F }` exists; or
2. no `SchedulerServicedFrame { frame: F }` exists; or
3. the tick for frame `F` occurs at or after `VBlankFired { frame: F + 1 }`; or
4. the scheduler service for frame `F` occurs at or after `VBlankFired { frame: F + 1 }`.

The first and last run frames are excluded because startup and shutdown may intentionally straddle widget initialization/completion.

For all gated frames:

```text
VBlankFired(F) < WidgetTickDispatched(F) < VBlankFired(F + 1)
VBlankFired(F) < SchedulerServicedFrame(F) < VBlankFired(F + 1)
```

The `<` relation above means lexicographic event order over `(mcycle_since_boot, seq)`.

`frame_service_misses` is therefore a project-level frame-service metric, not a claim that the hardware VBlank signal failed to occur.

Soft-margin reporting: for every gated frame the realism report records `cycles_remaining_at_widget_tick = vblank_fire_mcycle(F+1) - widget_tick_dispatch_mcycle(F)`. Mean, p99, min, and max across the run are reported. These are not gated; they inform M1's `soft_deadline_margin` calibration.

Also record `cycles_remaining_at_scheduler_service` using the same convention. This distinguishes "the ISR updated the widget" from "the cooperative runtime serviced the frame."

If a frame is missed, the corresponding margin is recorded as a signed negative value rather than omitted. This keeps failure reports diagnostic.

### 9.3 Frame-service widget

Resolved decision: tile-based frame-service widget, not sprite-based.

The widget renders a four-character hexadecimal frame counter in the top-left tile region. It updates through the runtime’s `video_commit` path. It does not write directly to VRAM outside legal LCD/VBlank conditions.

The display invariant is:

```text
displayed_value == vblank_count mod 0x10000
```

within the gated frame interval.

This widget proves frame-service liveness, not matmul progress. Compute progress is tracked separately through `compute_progress_epoch` and the `ComputeProgressEpochAdvanced` frame event.

### 9.4 What happens if the gate fails

If `N = 128` cannot hit the gate after iteration, the L4 bead does not close. The author iterates on (in priority order):

1. **Yield granularity.** Drop from `KLaneFullTile` (256 products per quantum) to `KLaneRows4` (64 products per quantum). Costs cycles, gains schedulability.
2. **Yield granularity again.** Drop from `KLaneRows4` to `KLaneRow` (16 products per quantum). Costs more cycles, maximizes schedulability without changing tile shape.
3. **Multiply table locality and address-formation cost.** Confirm Bank0 residency, per-step access pattern, and table-index address formation.
4. **Scheduler policy.** Re-tune the cooperative scheduler's `FrameBudget` / `soft_deadline_margin` defaults (recorded in `runtime_knobs`).

Each iteration step is recorded in `docs/review/f-b1/l4-iterations.md` and summarized in the L4 reviewer checklist. The realism report records the chosen knob set as part of its provenance.

Tile-shape changes such as `8×8×16` or `8×8×8` are not in-scope F-B1 knobs. If even `KLaneRow` plus scheduler-policy tuning cannot meet the gate, smaller tiles become an architectural exit and a load-bearing finding for M1.

If, after exhausting the in-scope knobs, the gate cannot hold at `N = 128`, the milestone surfaces a real architectural cost ceiling. The bead remains open until an explicit RFC amendment changes the acceptance target and documents (a) the largest `N` for which the gate holds, (b) the reason it cannot hold at `N = 128`, and (c) the implication for M1's `QuantSpec` choice. The waiver is an amended closure path, not normal F-B1 closure.

## 10. `realism_report.v1.json`

### 10.1 Location

Schema/types live in `gbf-report`.

Frozen review artifact:

```text
docs/review/f-b1/artifacts/realism_report.v1.json
```

### 10.2 Units

Unless the field name says otherwise:

* cycle fields are M-cycles
* byte fields are bytes
* frame fields are VBlank frames
* time fields with `_seconds_f64` are derived from DMG clock, not host wall time

No report field is based on host wall-clock timing.

`git_dirty` is evaluated before artifact writes in a fresh checkout. Regeneration is valid only if the final worktree diff is clean after the script completes.

Canonical JSON rule:

* UTF-8 JSON object keys are emitted in lexicographic order at every object level;
* no insignificant whitespace is emitted;
* all integer fields are emitted as base-10 JSON numbers;
* all floating-point fields must be finite and are emitted using one deterministic shortest-roundtrip formatter;
* arrays whose order is semantically meaningful are explicitly specified; `runs` is sorted by ascending `n`;
* fields with unknown or unmeasured values are rejected for the checked-in report rather than encoded as `0`, `null`, or omitted.

Schema fixtures may use small synthetic values. The checked-in `docs/review/f-b1/artifacts/realism_report.v1.json` must satisfy the semantic validator, not only the JSON shape validator.

### 10.3 Top-level shape

```json
{
  "schema": "realism_report.v1",
  "headline_n": 128,
  "run_order": [32, 64, 96, 128],
  "toolchain_identity": {
    "rustc": "rustc 1.92 ...",
    "host_triple": "...",
    "source_date_epoch": 0,
    "target_profile_hash": "sha256:...",
    "emulator_adapter_hash": "sha256:...",
    "gameroy_core_hash": "sha256:..."
  },
  "reproducibility": {
    "git_sha": "...",
    "git_dirty": false,
    "report_self_hash": "sha256:...",
    "regenerated_from_pinned_inputs": true
  },
  "workload": {
    "kind": "MatmulI8Bringup",
    "sizes": [32, 64, 96, 128],
    "tile": { "m": 16, "n": 16, "k": 16 },
    "operand_fixture": "DeterministicAffineV1",
    "operand_layout": {
      "a": { "bank": 1, "offset": 0 },
      "b": { "bank": 2, "offset": 0 },
      "multiply_table_bank": 0
    }
  },
  "runtime_knobs": {
    "yield_quantum": "KLaneFullTile",
    "soft_deadline_margin_mcycles": 0,
    "scheduler_profile": "Bringup"
  },
  "runs": [
    {
      "n": 128,
      "build_identity": {
        "build_hash": "sha256:...",
        "runtime_nucleus_hash": "sha256:...",
        "compute_bringup_request_hash": "sha256:...",
        "asm_ir_hash": "sha256:...",
        "rom_sha256": "sha256:...",
        "operand_fixture_hash": "sha256:...",
        "quarter_square_table_hash": "sha256:..."
      },
      "structural_counts": {
        "products": 2097152,
        "output_tiles": 64,
        "k_tiles_per_output_tile": 8,
        "operand_panel_copies": 1024,
        "operand_panel_bytes_copied": 262144,
        "full_output_bytes": 65536
      },
      "compute_costs": {
        "cycles_per_product_accumulate": { "sample_count": 0, "min": 0, "mean": 0, "max": 0, "p99": 0 },
        "cycles_per_output_tile": { "sample_count": 0, "min": 0, "mean": 0, "max": 0, "p99": 0 },
        "cycles_per_full_matmul": 0,
        "max_unyielded_compute_mcycles": 0,
        "wall_clock_seconds_at_gb_clock_f64": 0.0
      },
      "memory_bandwidth": {
        "rom_bank0_table_bytes_read": 0,
        "romx_operand_bytes_read": 0,
        "effective_bank0_table_read_bandwidth_bytes_per_sec_f64": 0.0,
        "effective_romx_operand_read_bandwidth_bytes_per_sec_f64": 0.0,
        "wram_peak_bytes": 0,
        "sram_peak_bytes": 0,
        "hram_peak_bytes": 0
      },
      "rom_layout": {
        "bank0_used_bytes": 0,
        "bank0_free_bytes": 0,
        "romx_operand_bank_size_bytes": 16384
      },
      "banking": {
        "logical_bank_switches_per_output_tile": 0,
        "logical_bank_switches_per_full_matmul": 0,
        "mbc_rom_bank_register_write_count": 0,
        "bank_lease_acquire_count": 0,
        "bank_lease_release_count": 0,
        "bank_lease_balance": 0,
        "max_active_bank_leases": 0,
        "yield_while_bank_lease_active_count": 0,
        "harness_pause_while_bank_lease_active_count": 0,
        "max_bank_lease_hold_mcycles": 0
      },
      "scheduling": {
        "frames_to_completion": 0,
        "gated_frame_start": 0,
        "gated_frame_end_exclusive": 0,
        "gated_frame_count": 0,
        "vblank_count": 0,
        "widget_update_count": 0,
        "scheduler_service_count": 0,
        "frame_service_misses": 0,
        "max_no_progress_frames": 0,
        "worst_case_interrupt_latency_mcycles": 0,
        "cycles_remaining_at_widget_tick": {
          "min": 0,
          "mean": 0,
          "max": 0,
          "p99": 0
        },
        "cycles_remaining_at_scheduler_service": {
          "min": 0,
          "mean": 0,
          "max": 0,
          "p99": 0
        }
      },
      "conformance": {
        "byte_exact_match": true,
        "reference": "gbf-verify::matmul_reference_i8",
        "reference_source_hash": "sha256:...",
        "fixture_hash": "sha256:...",
        "expected_output_hash": "sha256:..."
      }
    }
  ]
}
```

### 10.4 Self-hash rule

`report_self_hash` is computed over canonical JSON with `report_self_hash` set to:

```text
sha256:0000000000000000000000000000000000000000000000000000000000000000
```

The report emitter writes the final hash afterward.

The semantic validator recomputes the self-hash using the same canonical JSON routine and rejects the report if the stored hash differs.

The self-hash is computed after all run identities, artifact hashes, and derived metrics have been finalized. No field may be mutated after `report_self_hash` is written.

### 10.5 Sanity expectations

Structural counts are gates. Cycle, rate, and wall-clock-at-GB-clock values are measured, not pre-gated.

For `N = 128`:

| Field                   | Expected order                      |
| ----------------------- | ----------------------------------- |
| products                | exactly `2,097,152`                 |
| output tiles            | exactly `64`                        |
| K-tiles per output tile | exactly `8`                         |
| operand panel copies    | exactly `64 × 8 × 2 = 1,024`        |
| operand bytes copied    | about `1,024 × 256 = 262,144` bytes |
| Bank0 table bytes read  | about `2 × products × 2 = 8,388,608` bytes, before caching/micro-optimizations |
| bank lease acquires     | about `1,024`                       |
| bank lease releases     | same as acquires                    |
| bank lease balance      | exactly `0`                         |
| full output bytes       | exactly `65,536`                    |
| frame service misses    | exactly `0`                         |
| byte exact match        | exactly `true`                      |

Cycle numbers are expected to be large and measurement-defined. The purpose of F-B1 is to stop guessing.

The report semantic validator rejects:

* a missing run for any `N ∈ {32, 64, 96, 128}`;
* duplicate runs for the same `N`;
* a run order other than ascending `N`;
* `products != N³`;
* `output_tiles != (N / 16)²`;
* `k_tiles_per_output_tile != N / 16`;
* `operand_panel_copies != output_tiles × k_tiles_per_output_tile × 2`;
* `operand_panel_bytes_copied != operand_panel_copies × 256`;
* `full_output_bytes != N × N × 4`;
* `bank_lease_balance != 0`;
* `byte_exact_match != true`;
* `sample_count == 0` for checked-in measured statistics.

## 11. Validation strategy

### 11.1 Reference correctness

Claims:

* reference accepts only valid square dimensions
* reference output is row-major
* reference uses exact i32 accumulation
* verifier fixture materialization is deterministic
* production and verifier fixture materializers agree byte-for-byte

Gates:

```text
gbf-verify::matmul_reference_i8_known_fixture
gbf-verify::matmul_reference_rejects_bad_shape
gbf-verify::fixture_generation_is_stable
gbf-verify::output_bytes_are_little_endian_i32
gbf-codegen::operand_fixture_matches_verify_for_review_sizes
```

### 11.2 Table correctness

Claims:

* table has exactly 512 entries
* serialized table is 1,024 bytes
* production and reference generators match
* table multiplication equals direct multiplication over all i8 pairs

Gates:

```text
gbf-codegen::quarter_square_table_shape
gbf-codegen::quarter_square_table_matches_verify
gbf-codegen::quarter_square_mul_exhaustive_i8
```

### 11.3 Request and plan correctness

Claims:

* invalid dimensions rejected
* non-16 tile rejected
* same-bank operands rejected
* bank 0 operands rejected
* invalid operand offsets rejected
* request hash changes when fixture spec, bank, or offset changes
* plans are deterministic

Gates:

```text
gbf-codegen::compute_bringup_request_accepts_valid
gbf-codegen::compute_bringup_request_rejects_same_bank
gbf-codegen::compute_bringup_request_rejects_bank0_operand
gbf-codegen::compute_bringup_request_rejects_bad_operand_offset
gbf-codegen::compute_bringup_request_rejects_bad_tile
gbf-codegen::compute_bringup_request_hash_includes_fixture_and_layout
gbf-codegen::rom_window_plan_is_deterministic
gbf-codegen::rom_window_plan_bank0_resources
```

### 11.4 Banking correctness

Claims:

* generated compute code does not write MBC registers directly
* every operand copy is bracketed by acquire/release
* lease balance is zero
* only runtime banking code performs MBC writes

Gates:

```text
gbf-codegen::generated_code_has_no_raw_mbc_writes
gbf-codegen::operand_copy_requires_active_lease
gbf-runtime::banklease_balance_for_f_b1_trace
gbf-runtime::no_yield_while_banklease_active
gbf-asm::effect_classification_flags_direct_mbc_writes
```

### 11.5 Scheduler correctness

Claims:

* progress epoch advances during sustained compute
* `no_progress_frames` remains bounded
* widget ticks once per gated frame
* no missed cooperative frame service in headline run

Gates:

```text
gbf-runtime::liveness_advances_per_yield_quantum
gbf-emu::frame_trace_widget_tick_ordering
gbf-emu::f_b1_l4_n128_no_frame_service_misses
gbf-emu::f_b1_l4_max_no_progress_frames_le_1
```

### 11.6 Report correctness

Claims:

* schema validates the checked-in report
* all fields are mandatory
* semantic invariants validate the checked-in report
* report generation is deterministic
* self-hash convention is stable

Gates:

```text
gbf-report::realism_report_v1_accepts_checked_fixture
gbf-report::realism_report_v1_rejects_missing_fields
gbf-report::realism_report_v1_rejects_structural_count_mismatch
gbf-report::realism_report_v1_rejects_zero_sample_count_in_checked_report
gbf-report::realism_report_v1_self_hash_round_trip
./scripts/review/f-b1/regen.sh
./scripts/review/f-b1/verify-packet.sh
```

## 12. Claim-to-gate matrix

| Claim                                                            | Gate                                                         |
| ---------------------------------------------------------------- | ------------------------------------------------------------ |
| F-B1 uses exact `i8 × i8 → i32` semantics                        | `gbf-verify::matmul_reference_i8_known_fixture`              |
| Reference shape validation rejects invalid matrices              | `gbf-verify::matmul_reference_rejects_bad_shape`             |
| Quarter-square table has 512 entries / 1,024 bytes               | `gbf-codegen::quarter_square_table_shape`                    |
| Quarter-square multiply matches direct multiply for all i8 pairs | `gbf-codegen::quarter_square_mul_exhaustive_i8`              |
| Production and reference table generators agree                  | `gbf-codegen::quarter_square_table_matches_verify`           |
| `ComputeBringupRequest` rejects same-bank operands               | `gbf-codegen::compute_bringup_request_rejects_same_bank`     |
| `ComputeBringupRequest` rejects bank 0 operands for A/B          | `gbf-codegen::compute_bringup_request_rejects_bank0_operand` |
| Tile size is fixed at 16×16×16                                   | `gbf-codegen::compute_bringup_request_rejects_bad_tile`      |
| Kernel and table are Bank0-resident                              | `gbf-codegen::rom_window_plan_bank0_resources`               |
| Generated code uses BankLease/BankGuard                          | `gbf-codegen::f_b1_l2_lowering_uses_banklease_ops`           |
| Generated code contains no raw MBC writes                        | `gbf-codegen::f_b1_l2_generated_code_has_no_raw_mbc_writes`  |
| Lease acquire/release balance is zero                            | `gbf-runtime::f_b1_l2_banklease_balance`                     |
| No yield occurs while a bank lease is active                     | `gbf-runtime::f_b1_l2_no_yield_while_banklease_active`       |
| L0 WRAM smoke matches reference                                  | `gbf-emu::f_b1_l0_wram_matmul_dump_matches_reference`        |
| L1 compiled path matches reference                               | `gbf-emu::f_b1_l1_compiled_matmul_matches_reference`         |
| L2 cross-bank path matches reference                             | `gbf-emu::f_b1_l2_cross_bank_matmul_matches_reference`       |
| L3 streamed N=128 output matches reference                       | `gbf-emu::f_b1_l3_streamed_output_matches_reference_n128`    |
| Widget event ordering proves no late ticks                       | `gbf-emu::frame_trace_widget_tick_ordering`                  |
| L4 headline run has no frame-service misses                      | `gbf-emu::f_b1_l4_n128_no_frame_service_misses`              |
| L4 headline run remains live                                     | `gbf-emu::f_b1_l4_liveness_no_progress_frames_bounded`       |
| Report schema accepts checked-in report                          | `gbf-report::realism_report_v1_accepts_checked_fixture`      |
| Report regeneration is deterministic                             | `./scripts/review/f-b1/verify-packet.sh`                     |
| Review packet is reproducible                                    | `./scripts/review/f-b1/regen.sh` then clean diff             |

## 13. Task graph

```text
F-B1 Compute Bringup
├── T-B1.1 L0 in-WRAM smoke
├── T-B1.2 L1 request-to-ASM pipeline
│   └── depends on T-B1.1
├── T-B1.3 L2 cross-bank operands via BankLease
│   └── depends on T-B1.2 and F-A4
├── T-B1.4 L3 full tiling and streamed output
│   └── depends on T-B1.3
├── T-B1.5 L4 cooperative yielding and frame gate
│   └── depends on T-B1.4
└── T-B1.6 L5 realism report
    └── depends on T-B1.5
```

F-B1 ships as one PR. The PR history should be layer-ordered, but `main` should receive only the complete integrated milestone.

## 14. Review packet

Required path:

```text
docs/review/f-b1/
```

Required files:

```text
scope.md
architecture.md
claim-to-gate.md
reviewer-checklist.md
known-debt.md
l4-iterations.md
reproducibility.md
generated-artifacts.md
artifacts/realism_report.v1.json
artifacts/matmul_n128.gb
artifacts/matmul_n128.sym
artifacts/matmul_n128.lst
artifacts/matmul_n128.sha256
artifacts/matmul_n128.map.json
artifacts/frame_trace_n128.json
artifacts/tile_dump_index_n128.json
artifacts/cycle_attribution_n128.json
```

Required scripts:

```text
scripts/review/f-b1/regen.sh
scripts/review/f-b1/verify-packet.sh
```

Hard rule:

> A fresh checkout regenerates the packet with one command, and staleness fails loudly.

Long-running emulator/bench gates are owned by:

```text
./scripts/review/f-b1/regen.sh
./scripts/review/f-b1/verify-packet.sh
```

Unit tests may include ignored versions of the `N = 128` gates, but ordinary `cargo test` must not unexpectedly run the full headline benchmark.

## 15. Risks and what we want to learn

### 15.1 Risk table

| Risk                                        | Mitigation                                                               |
| ------------------------------------------- | ------------------------------------------------------------------------ |
| F-A4 lands late                             | L0/L1 can proceed; F-B1 cannot close until L2+ use real BankLease        |
| K-lane yield quantum too coarse             | Allow `KLaneRows4`; if still too coarse, allow `KLaneRow`; record chosen quantum |
| N=128 frame gate fails                      | Do not close without waiver; report ceiling honestly (§9.4)              |
| Bank0 kernel too large                      | Surface as M1 input; do not move switching kernel into ROMX              |
| Report nondeterminism                       | No host wall-clock fields; canonical JSON; self-hash rule                |
| Skeletal types overcommit M1                | Use F-B1-specific names and explicit M1 deferred comments (§7)           |
| Full benchmark too heavy for ordinary tests | Keep unit tests small; review packet script owns full N=128 regeneration |
| Harness dump semantics drift                | Use existing F-A3 dump operation; do not add ad hoc ABI                  |
| Bit-reproducibility fails to host-clock leak | Report records only emulated cycle counts and emulator-deterministic outputs |
| Frame gate passes because ISR updates widget but scheduler starves | Gate requires `SchedulerServicedFrame` for every gated frame, not just widget updates |
| L0/L1 smoke layouts drift into closure artifacts | `WramSmoke` is allowed only for smoke profiles; L5 report must use distinct ROM banks |
| Production crates depend on `gbf-verify` shape types | Shared shape types live outside `gbf-verify`; verifier depends on shared types, not vice versa |
| Bank0 layout overflows silently | ROM layout reports Bank0 free bytes and fails if kernel/table/runtime overlap reserved regions |

### 15.2 First-principles structural counts that F-B1 is testing

The realism report replaces every cycle estimate with a measurement. This subsection states only the *structural* counts that flow from the chosen tile schedule and operand layout:

* **Product-accumulate via quarter-square table:** measurement-defined. The logical operation is two table lookups, one `i16` subtract, sign-extension, and one `i32` accumulator add. The LR35902 cost is expected to be dominated by address formation, memory traffic, carry propagation, and loop overhead.
* **Inner product over `K = 128`:** 128 product-accumulate steps per output element.
* **Output element count at `N = 128`:** 16,384.
* **Per-matmul compute:** measurement-defined. The structural product count is exactly `128³ = 2,097,152`.
* **Operand panel copies:** `64 output tiles × 8 k-tiles × 2 panels = 1,024` panel copies. Each panel is 256 bytes, so operand panel traffic is `262,144` bytes.
* **Bank0 table traffic:** two 16-bit table reads per product implies roughly `8,388,608` Bank0 table bytes read before any micro-optimization.
* **Estimated wall-clock:** intentionally not asserted here. F-B1 exists to measure it.

These counts let reviewers sanity-check the realism report's structural fields (products, output tiles, K-tiles, panel copies, lease counts). Cycle and wall-clock numbers come exclusively from the report.

### 15.3 What we will learn from the realism report

Concrete inputs to M1's design:

| Realism report field                                | M1 design input it feeds                                                              |
| --------------------------------------------------- | ------------------------------------------------------------------------------------- |
| `cycles_per_product_accumulate` mean                | Product-accumulate cost in the dense kernel; sets the absolute floor for cycles per token |
| `cycles_per_full_matmul`                            | Wall-clock budget per dense layer; informs M1's `D_model` and layer-count choices     |
| `effective_bank0_table_read_bandwidth_bytes_per_sec_f64` | Sets M1's fixed-bank table-read budget per tile                                  |
| `effective_romx_operand_read_bandwidth_bytes_per_sec_f64` | Sets M1's switchable-ROM operand-read budget per tile                            |
| `logical_bank_switches_per_full_matmul`             | Sets M1's `RomWindowPlan` re-residency budget                                         |
| `cycles_remaining_at_widget_tick.p99`               | Calibrates M1's `soft_deadline_margin` default                                        |
| `cycles_remaining_at_scheduler_service.p99`         | Calibrates cooperative scheduler service margin separately from ISR/widget margin     |
| `worst_case_interrupt_latency_mcycles`              | Sets the `InterruptPolicy::ShortCriticalSection` budget concretely                    |
| `max_unyielded_compute_mcycles`                     | Bounds how much compute can run before cooperative service is possible                |
| `max_bank_lease_hold_mcycles`                       | Bounds scheduler starvation caused by non-yieldable banked operand copies             |
| `wram_peak_bytes`                                   | Headroom available for M1's larger accumulators / temporaries                         |
| `frames_to_completion`                              | Token-latency baseline; M1 budgets per-token frames against this                      |

If the report says M1's quantised dense kernel will take >5 s per token at the design size, M1 must either reduce model size, increase quantisation aggressiveness, or accept the latency. That decision is data-driven by F-B1's report rather than guessed.

## 16. Resolved seed open questions

| Seed question                     | Resolution                                                         |
| --------------------------------- | ------------------------------------------------------------------ |
| Multiply table ROM vs HRAM mirror | Bank0 ROM only; no HRAM mirror in F-B1                             |
| Kernel bank                       | Bank0                                                              |
| DumpRegion payload sizing         | No new `DumpRegion`; use existing F-A3 dump operation              |
| Multiply table residency epoch    | Named `PermanentBank0`, no numeric epoch sentinel                  |
| Reference API shape               | checked matrix view/newtype API in `gbf-verify`                    |
| Widget rendering                  | tile-based four-hex-digit counter                                  |
| Mermaid diagram                   | include if useful in `architecture.md`; not a gate                 |
| Numeric report expectations       | report exact structural counts; cycle values are measured          |
| Scheduler knobs in report         | yes, record `runtime_knobs`                                        |
| Table generator location          | production in `gbf-codegen`, independent reference in `gbf-verify` |

## 17. End state

After F-B1, M1 starts with:

* a working sustained-compute ROM path
* real BankLease pressure from cross-bank operands
* a deterministic host reference pattern
* a working streamed-output harness pattern
* a VBlank/frame gate already proven under load
* concrete measured costs for integer product-accumulate, ROM reads, bank switching, and scheduler yield cadence
* a reproducible `realism_report.v1.json` baseline

F-B1 does not make the system model-aware. It makes the system compute-real.

## 18. References

- `history/planv0.md` — milestone sequence (M0..M6) and engineering rules.
- `history/rfcs/F-A1-gbf-asm.md` — encoder, `MachineEffect`, `PrivilegeClass`, `SystemCallKind`, `BankLease`/`BankRelease` system-call surface.
- `history/rfcs/F-A2-gbf-hw.md` — `TargetProfileId`, MBC5 register addresses, calibration schema.
- `history/rfcs/F-A3-gbf-abi.md` — `HarnessOp` (existing arena/dump operation reused by F-B1), `BuildIdentityBlock`, `LivenessCounters`, `ResourceLease`, `ResourceLeaseKind`, `InterruptPolicy`, `SemanticCheckpointId`, `TraceEvent`.
- `history/rfcs/F-A4-banklease-banking.md` — `BankLease`/`BankGuard` runtime ABI, HRAM shadow, MBC5 acquire/release lowering, `InterruptSafetyKind`.
- `history/rfcs/F-A6-*.md` — deferred `gbf-migrate` scaffolding (not load-bearing for F-B1).
- `history/glossary.md` — banking, lease, residency, slice, yield, liveness, denotational stratum, artifact stratum, operational stratum.
- `CONSTITUTION.md` — Doctrine of Correctness (§I), Velocity of Tooling (§II), Shifting Left (§III), Immutable Runtime (§IV), Observability (§V), Knowledge Graph (§VI).
- `CLAUDE.md` — beads workflow, pre-commit hook, session protocol, project skills.
