# GBLLM Project Glossary

Status: Draft
Date: 2026-04-29

This glossary establishes shared vocabulary for GBLLM design work. The first
terms focus on RFC F-A1, `gbf-asm`, and the backend boundary. Each entry says
what the term means, what it does not mean, and where the owner boundary lives.

Status markers:

- Current code: already implemented or represented in the repo.
- RFC term: proposed by the F-A1 RFC and should be made precise before code.
- Future owner: intentionally owned by a later feature or epic.

## Cross-Project Terms

### Bead

Status: Current workflow.

A tracked unit of work in `beads_rust`, stored in `.beads/issues.jsonl`.
Feature beads, task beads, and epic beads are planning and acceptance objects.
They are not implementation modules.

For F-A1:

- `bd-ssm` is the feature bead.
- `bd-11e` through `bd-1gc` are child task beads.

Use bead ids when discussing scope, acceptance criteria, closure evidence, and
dependencies.

### Epic

Status: Current workflow.

A bead that groups a large architectural area. Epic A (`bd-14y`) is the M0
foundation stack. Epic B owns the compiler pipeline. Epic F owns reports and
certificates. Epics describe ownership and dependency shape; they do not imply a
single crate or single PR.

### Feature

Status: Current workflow.

A bead-sized capability that usually spans one crate or one tightly coupled
slice. F-A1 is a feature because it completes `gbf-asm` as an authoring,
layout, encoding, and ROM-byte foundation.

### Task

Status: Current workflow.

A smaller bead that should be implementable and reviewable in one focused
change. T-A1.1 through T-A1.9 are tasks under F-A1.

### Contract

Status: Current design term.

A durable interface or invariant other crates are allowed to rely on. A
contract is stronger than an implementation detail. Examples:

- `Instr` shapes are the contract consumed by encoder, layout, and cycle model.
- `MachineEffect` is the contract later consumed by `ReachabilityValidation`.
- `gbf-abi` owns live execution memory layouts.

When we call something a contract, tests and closure evidence should pin it.

### Owner

Status: Current design term.

The crate, module, feature, or bead responsible for deciding a concept's public
shape. Ownership matters because GBLLM deliberately avoids parallel definitions.

Examples:

- `gbf-hw` owns hardware constants.
- `gbf-asm::encoder` owns instruction bytes.
- F-A4 owns the real BankLease / BankGuard ABI.
- Epic B owns whole-program reachability validation.

### Boundary

Status: Current design term.

The line where responsibility changes hands. Boundaries are load-bearing in
this project because they prevent semantic drift between crates.

For F-A1, the important boundaries are:

- builder authoring versus layout;
- symbolic section IR versus concrete machine IR;
- structured op intent versus runtime ABI lowering;
- instruction typing versus byte encoding;
- instruction-local effect classification versus whole-program proof.

### Semantic

Status: Current design term.

Use this word carefully. In the plan, "semantic" does not mean "whatever is
important." It refers to meaning at a named stratum.

Avoid saying "semantic" when you mean "runtime," "layout," "address," "byte,"
"debug," or "operational." For F-A1, most work is operational/backend work, not
model semantics.

### Denotational Stratum

Status: Current design term. Future owner: `gbf-oracle`.

Target-independent reference meaning, usually evaluated by the
`DenotationalOracle` from a `ReferenceProgram`. It is not tied to Game Boy
layout, banks, timing, or assembly.

F-A1 does not implement this stratum.

### Artifact-Semantic Stratum

Status: Current design term. Future owner: `gbf-oracle` and `gbf-artifact`.

The frozen deployed artifact's canonical logical behavior after export and
quantization, but before schedule, layout, bank assignment, and runtime
execution.

F-A1 does not implement this stratum.

### Operational Stratum

Status: Current design term.

The scheduled/runtime/assembly execution level. F-A1 operates here: sections,
machine effects, bank placement, encoded bytes, cycle costs, and ROM images are
operational objects.

## F-A1 Ownership Terms

### `gbf-asm`

Status: Current crate.

The typed LR35902 assembly eDSL plus supporting backend machinery: instruction
types, sections, symbols, provenance, builder API, effect classification, cycle
model, layout, relaxation, encoder, listings, and ROM builder.

It is not:

- a text assembler;
- a whole compiler backend;
- the runtime BankLease implementation;
- a hardware constants crate.

### LR35902 / SM83

Status: Current hardware term.

The Game Boy CPU instruction set target for `gbf-asm`. The RFC uses LR35902
because that is the common project wording; Pan Docs and gbdev opcode tables
also refer to the related SM83 vocabulary.

### eDSL

Status: Current design term.

An embedded domain-specific language in Rust. `gbf-asm::Builder` is an eDSL
because Rust code emits structured assembly IR through typed methods rather than
writing assembly text or raw bytes.

### Authoring Layer

Status: Current design term.

The surface through which executable code is intentionally created. For GBLLM,
generated executable code must originate from `AsmIR`, `Instr`, or audited
runtime builders. Ad hoc byte pushes are not an authoring layer.

For F-A1, the authoring layer is the builder plus typed section items.

### AsmIR

Status: Current design term, partly represented in current code.

The structured assembly-level IR authored by `Builder` and consumed by layout,
relaxation, reachability, encoder, listings, and ROM assembly.

In current code, AsmIR is represented by:

- `Section`;
- `SectionItem`;
- `Instr`;
- `PreLayoutOp`;
- `LegalizationOp`;
- `SymbolName`;
- `InstrProvenance`;
- `MachineEffect` classification.

AsmIR is not raw encoded bytes.

### `Instr`

Status: Current code.

The canonical concrete LR35902 instruction enum in `gbf-asm/src/isa.rs`.
Concrete means it represents a legal instruction shape with concrete operands,
such as `JrRel { off: i8 }` or `Call { addr: u16 }`.

`Instr` does not own:

- labels;
- unresolved symbols;
- structured ops;
- relocations;
- branch relaxation;
- final opcode bytes.

The encoder consumes `Instr`. Layout and relaxation may create or replace
`Instr` values, but should not add symbolic state inside `Instr`.

### Legal CPU Encoding

Status: Current design distinction.

An encoding the LR35902 CPU can execute. Legal CPU encodings are defined by the
hardware, Pan Docs, and opcode tables.

Not every legal CPU encoding is necessarily part of GBLLM's canonical authoring
surface.

### Canonical Project Encoding

Status: Current design distinction.

The encoding form GBLLM chooses as its normal representation when multiple legal
CPU encodings could express a similar operation. Example: high-memory A
transfers use `LDH` forms rather than longer absolute forms. In current code,
`DirectAddr` rejects `$FF00..=$FFFF` so this choice is structural.

When canonical project encoding rejects or redirects a legal CPU encoding, the
boundary must be documented and tested.

### Operand

Status: Current code.

A typed description of what an instruction reads or writes. Current code has
broad helper operands (`Operand8`, `Operand16`) and narrower instruction
operands (`AluSrc8`, `IncDec8Target`, `CbTarget`, `Reg16Data`,
`Reg16Stack`, `Reg16Addr`).

Do not use one broad operand enum at every instruction boundary if it would make
invalid instruction shapes representable.

## Section And Symbol Terms

### `Section`

Status: Current code.

A named container of symbolic pre-layout assembly items. A section has:

- a `SectionId`;
- a `SectionRole`;
- a canonical `SymbolName`;
- a `SectionPrivilege`;
- ordered `SectionItem`s;
- alignment and optional size hint.

A `Section` is not yet placed in a concrete ROM bank or address.

### `SectionItem`

Status: Current code.

An item inside a `Section`. Current variants are:

- `Label`;
- `Instr`;
- `Db`;
- `Dw`;
- `Align`;
- `PreLayoutOp`;
- `LegalizationOp`;
- crate-private `Raw`.

Every item carries `InstrProvenance`. Items with unknown pre-layout width must
return `None` or an explicitly named lower/upper bound, not pretend to be zero
bytes.

### `SectionRole`

Status: Current code.

The role or residency class of a section. Current roles include
`Bank0Nucleus`, `CommonBank`, `ExpertBank`, WRAM/HRAM/SRAM/VRAM/OAM roles, and
`HeaderCartridge`.

`SectionRole` is a declaration used by placement and reachability. It is not by
itself a proof that code is reachable only from safe contexts. Later
`ReachabilityValidation` computes that proof.

### `SectionPrivilege`

Status: Current code.

The section-level policy that controls which effect and privilege classes may
be emitted into a section. It validates emitted items and revalidates existing
items if the section privilege changes.

It is a local guard, not a whole-program reachability proof.

### Label

Status: Current code.

A marker inside a section that binds a symbol name to a section-local offset
after layout. A label has zero encoded bytes.

Labels are not enough by themselves to model symbolic branches unless the
branch item retains the target symbol until relaxation.

### `SymbolName`

Status: Current code.

A canonical dot-separated symbol name built from validated lowercase ASCII
segments. Examples include runtime, kernel, expert, and section symbols.

Callers must not join raw strings with dots and validate afterward, because that
allows collisions. Build names from validated segments.

### `SymbolTable`

Status: Current code.

A deterministic mapping from `SymbolName` to resolved post-layout address
metadata. Names are unique. Multiple names may alias the same address.

Reverse lookup must return all names for an address, not just one primary name.

### `.sym`

Status: RFC term.

A symbol-map output consumed by debuggers/emulators such as BGB and by harness
tooling. It should be deterministic and derived from the final symbol table.

## Pipeline Phase Terms

### Symbolic Pre-Layout IR

Status: Current design term.

The phase represented by `Section` before final addresses are known. It may
contain labels, alignments, structured op intent, raw escape hatches, data
directives, and concrete `Instr` values. Current code represents structured op
intent with phase-named `PreLayoutOp` and `LegalizationOp` section items.

It may not claim final byte offsets or final branch distances.

### Symbolic Branch

Status: RFC term, not current code.

A proposed section item that keeps a target `SymbolName` until layout and
relaxation can decide whether to emit `JR`, `JP`, `CALL`, or a far-call thunk.

Important current-code fact: `Instr::JrRel` and `Instr::Call` are concrete and
already require offsets/addresses. Any RFC language saying "Builder emits
symbolic branches" describes required new work, not the current implementation.

### Relocation

Status: RFC term.

The act of resolving a symbolic reference into a concrete address or offset.
For F-A1, relocation should be a layout/relaxation concern, not an encoder
side effect.

### Lowering

Status: Current design term, but overloaded.

Lowering means translating from a higher-level representation to a lower-level
representation. Because this word is easy to overload, use a qualified term:

- pre-layout op lowering;
- legalization op lowering;
- branch relaxation;
- byte encoding;
- ROM assembly.

Do not say only "lowering" when the phase boundary matters.

### Pseudo-Op

Status: Historical/plan term.

Umbrella term for structured assembly intent that is not one LR35902
instruction. Use this only when referring to older `planv0.md` language or to
both phase-specific classes together.

For new RFC text, prefer the phase-specific terms:

- `PreLayoutOp`;
- `LegalizationOp`.

### Structured Op

Status: RFC umbrella term.

An authoring-level assembly operation that is not a single concrete `Instr`.
Use this when referring to the broad concept without implying lowering order.
When lowering order matters, use `PreLayoutOp` or `LegalizationOp`.

F-A1 should avoid introducing a new unqualified "pseudo-op lowering" concept.
The source API now uses `PreLayoutOp` and `LegalizationOp`, so the lowering
order is visible at the `SectionItem` boundary.

### `PreLayoutOp`

Status: Current code and RFC term.

A structured assembly op whose emitted form does not depend on final section
address, final bank placement, final branch width, or thunk placement. It lowers
before layout, producing ordinary section items inside a `LoweredSection`.

Examples, subject to F-A4 confirmation:

- `Yield`;
- `TraceProbe` when trace policy only decides emit-versus-strip;
- `BankLease` / `BankRelease` calls whose runtime ABI symbol is known without
  final placement.

A `PreLayoutOp` may still emit a symbolic call target. It just cannot need final
placement to choose its emitted shape.

### `LegalizationOp`

Status: Current code and RFC term.

A structured assembly op whose emitted form depends on final placement or on
the result of layout/relaxation. It is carried through layout as an explicitly
sized obligation and lowers during legalization, producing encoder-ready items
inside a `LegalizedSection`.

Expected examples:

- `FarCall`, because it needs to know caller/callee bank placement;
- any debug/assertion op whose emitted sequence depends on final bank, address,
  or thunk placement.

A `LegalizationOp` is not allowed to reach the encoder.

### Pre-Layout Op Lowering

Status: RFC term.

The pass that translates `PreLayoutOp` values before layout. It is not byte
encoding and should not live inside `encode_instr`.

### Legalization Op Lowering

Status: RFC term.

The legalization-time step that translates `LegalizationOp` values after final
placement facts are available. It is not byte encoding; it produces concrete
items for the encoder.

### `LoweredSection`

Status: RFC term, accepted F-A1 terminology.

Meaning: a section after `PreLayoutOp` lowering, but before final layout,
relaxation, and legalization. It may still contain labels, alignments,
symbolic branches, and `LegalizationOp` obligations.

It does not mean "ready for encoder." If a design needs byte-ready input, use
`LegalizedSection`.

### `LegalizedSection`

Status: RFC term, accepted F-A1 terminology.

Meaning: a section after layout, branch relaxation, far-call thunk insertion,
final alignment padding, and any remaining required expansion have produced
only encoder-ready concrete items.

This is the clean term for the encoder input.

A `LegalizedSection` should not contain:

- `PreLayoutOp`;
- `LegalizationOp`;
- unresolved labels as control-flow operands;
- unknown-width alignments;
- symbolic branch targets;
- pending thunk requests.

### `EncodedSection`

Status: RFC term, current open task.

The byte output for one placed and legalized section, plus stable offset
metadata mapping bytes back to source or legalized items.

It is not a full ROM image. It does not own cartridge header checksums or
power-of-two ROM padding.

### `PlacedSection`

Status: RFC term, current open task.

A section assigned to an address space, bank, start address, and size. Placement
is the output of layout and an input to symbol resolution, relaxation, encoding,
listings, and ROM assembly.

### `LayoutPlan`

Status: RFC term, current open task.

The deterministic set of `PlacedSection` decisions plus supporting placement
metadata such as bank count, free bytes, or thunk slots.

The layout plan records where things go. It should not encode instructions.

### `PlacedRom`

Status: Current plan term. Future owner: Epic B, with mechanisms from F-A1.

The compiler backend product after layout, label resolution, branch expansion,
and far-call legalization. It is richer than a set of bytes because it still
knows section structure, symbols, residency, and provenance.

F-A1 supplies the lower-level mechanisms. Epic B owns the full compiler-stage
`PlacedRom` product.

### `EncodedRom`

Status: Current plan term. Future owner: Epic B, with byte machinery from F-A1.

The final encoded ROM-side build product: `.gb`, `.sym`, `.lst`, and associated
metadata after all high-level decisions are frozen.

F-A1 owns deterministic byte production primitives. Epic B owns the full build
pipeline product.

## Structured Op And Runtime Terms

### `BankLease`

Status: Current `PreLayoutOp` variant. Future production owner: F-A4.

An authoring request to make a ROM or SRAM bank visible through the runtime
BankLease / BankGuard ABI. In `gbf-asm`, `BankLeaseSpec` records class, bank,
and lease id. It is authoring intent, not the final runtime ABI layout.

BankLease does not mean "write directly to an MBC5 register."

### `BankRelease`

Status: Current `PreLayoutOp` variant. Future production owner: F-A4.

The matching request to release a previously acquired bank lease. The builder
currently validates duplicate, unknown, and released lease lifecycle errors.

### `BankGuard`

Status: Future owner: F-A4.

The compile-time/runtime-ABI discipline that ensures bank leases are acquired
and released according to policy. In project language, this is the legal path to
bank state changes. The exact runtime shape is not owned by F-A1.

### Direct MBC Write

Status: Current effect term.

An instruction sequence that writes to cartridge control address ranges such as
MBC5 RAM enable, ROM bank low/high, or SRAM bank registers.

Direct MBC writes are privileged. Generated inference code should not emit
them. It should emit `BankLease` structured ops and let the runtime ABI handle
the writes.

### MBC5

Status: Current hardware target. Detailed owner: F-A2 / `gbf-hw`.

The cartridge memory bank controller target. Relevant facts:

- bank 0 is fixed at `$0000..=$3FFF`;
- one switchable ROM window is visible at `$4000..=$7FFF`;
- MBC5 supports 9-bit ROM bank numbers;
- external SRAM is visible at `$A000..=$BFFF` when enabled and banked.

`gbf-asm` may need minimal MBC5-aware types for layout and effects, but
hardware constants and register semantics belong in `gbf-hw`.

### `FarCall`

Status: Current `LegalizationOp` variant. Production details future-owned by F-A4/F-A5.

A call whose target may not be reachable through the currently visible ROM
bank. Far calls require bank visibility management and often a Bank0 thunk.

Do not treat `CALL addr` as a far call. LR35902 `CALL` only jumps within the
currently visible address space.

### Thunk

Status: RFC term.

A generated helper fragment that bridges a control-flow mismatch. In F-A1, the
most important thunk is a far-call thunk, usually Bank0-resident, that arranges
the correct bank visibility before transferring control.

The exact production thunk body is owned by the runtime/banking ABI, not the
encoder.

### `Yield`

Status: Current `PreLayoutOp` variant. Production owner: F-A5 scheduler / Epic B.

A request inserted at safe points so generated code can cooperatively return to
the scheduler. It is not a CPU interrupt by itself. It lowers to runtime
scheduler ABI code later.

### `TraceProbe`

Status: Current `PreLayoutOp` variant. Production owner: tracing/runtime/reporting
features.

A structured request to emit or collect operational trace information. It may
lower to code in trace builds or to zero bytes when disabled by policy.

### `AssertBank`

Status: Current `PreLayoutOp` variant, pending F-A4/F-A5 confirmation if any
address-specific assertion form is needed later.

A debug assertion that the observed bank state matches expected state. Release
builds may strip it. It should never become a hidden production bank-switching
mechanism.

### Stub Runtime / Stub Lowering

Status: RFC term.

A test/example-only implementation that lets F-A1 exercise the full pipeline
before F-A4/F-A5 exist. Stub outputs must be clearly non-production and should
not define public ABI names or byte layout contracts.

## Effect And Safety Terms

### `MachineEffect`

Status: Current code.

Instruction-local classification of what an instruction or structured op can do:
pure compute, memory loads/stores by region, dynamic address effects, stack
effects, control flow, interrupt control, MBC register writes, raw opaque bytes,
or runtime system calls.

Machine effects are facts available before whole-program analysis. They are not
reachability proofs.

### `MachineEffectKind`

Status: Current code.

The parameter-free class of a `MachineEffect`, used for section allowlists.
Example: all IO stores share `StoreToIo` as a kind even if the exact IO register
differs.

### `PrivilegeClass`

Status: Current code, with one F-A1 policy question.

The privilege required by a machine effect. Current values are `Normal`,
`Privileged`, and `InterruptHandler`.

Open F-A1 precision issue: a `BankLease` request should probably not grant the
same privilege as direct MBC register writes. We should either map bank-lease
structured op requests to `Normal` plus later reachability checks, or add a narrow
class such as `BankLeaseRequest`.

### Privileged

Status: Current code term.

A class of operation reserved for trusted runtime/banking/interrupt code. It
includes direct MBC writes and raw opaque bytes. Marking a section privileged is
a serious permission expansion.

### Interrupt Handler

Status: Current code term.

A privilege class for code that may execute as an ISR. `RETI` requires this
class. ISR safety also requires residency and bank-independence proofs later.

### Dynamic Address Effect

Status: Current code.

A load/store where instruction-local classification cannot know the concrete
memory region because the address comes from a register such as `HL`, `BC`, or
`DE`.

Dynamic effects are not automatically safe or unsafe. They create proof
obligations for later reachability/storage analysis.

### ReachabilityValidation

Status: Future owner: Epic B / F-B13.

The whole-program analysis that computes whether code/data is ISR-reachable,
yield-resume reachable, fault-path reachable, bank-lease protected, and so on.
It validates residency and privilege rules using the graph after branch
relaxation and thunk insertion.

F-A1 provides typed effects and structured sections. It does not replace this
analysis.

## Layout And Addressing Terms

### Address Space

Status: RFC term.

A named memory domain such as `Rom0`, `RomX`, `Wram`, `Hram`, `Sram`, `Vram`,
`Oam`, `Io`, or `Header`. This is more precise than saying "bank" for every
section, because not every section is ROM-backed.

### Bank

Status: Current hardware/layout term.

A numbered ROM or SRAM storage slice selected through fixed hardware rules.
Bank 0 is fixed ROM. Switchable ROM banks are visible through the `$4000..=$7FFF`
window. SRAM banks are visible through the `$A000..=$BFFF` window when enabled.

Do not use "bank" for WRAM, HRAM, VRAM, or OAM unless a type explicitly models
that as an address-space tag.

### Bank0

Status: Current project term.

The fixed 16 KiB ROM bank at `$0000..=$3FFF`. It contains the runtime nucleus,
interrupt vectors, scheduler, critical thunks, and other code/data that must be
bank-agnostic.

Bank0 space is scarce. Treat it as a safety-critical residency area, not a
general dumping ground.

### Switchable ROM Window

Status: Current hardware term.

The address range `$4000..=$7FFF`, where one selected ROM bank is visible at a
time. Calls into this window are only meaningful relative to the currently
selected bank.

### PlacementProfile

Status: Current plan term, RFC implementation term.

A policy family for placing sections into banks. The canonical names are
`StrictOnePerBank`, `Budgeted`, and `PackedExperts`.

F-A1 should implement deterministic placement mechanics. Epic B decides which
profile a build uses and why.

### Branch Relaxation

Status: RFC term.

The pass that replaces short branch encodings with wider legal encodings when
layout makes a target out of range, and that introduces far-call thunks when a
plain call cannot legally cross bank visibility.

Relaxation is not byte encoding. It produces concrete instructions or
legalization decisions for the encoder.

### Monotone Fixed Point

Status: RFC term.

A relaxation algorithm property: each iteration may only grow or preserve code
size, never shrink it. For example, `JR` may become `JP`, but not the reverse in
the same run. This makes convergence easier to reason about and test.

### `JR`

Status: LR35902 instruction term.

A short relative jump with an 8-bit signed offset from the instruction after the
`JR`. It is compact but limited in range.

### `JP`

Status: LR35902 instruction term.

An absolute 16-bit jump in the currently visible address space. It is larger
than `JR` but can reach any visible address.

### `CALL`

Status: LR35902 instruction term.

An absolute 16-bit subroutine call in the currently visible address space. It
does not by itself change ROM banks.

## Encoding And ROM Terms

### Encoder

Status: Current module stub, open task T-A1.7.

The only code that maps concrete `Instr` values to opcode bytes. It should not
own structured op policy, runtime ABI design, layout decisions, or ROM header
assembly.

### Byte Encoding

Status: RFC term.

The concrete sequence of opcode and immediate bytes emitted for one `Instr` or
for data directives. Byte encoding is deterministic and little-endian where the
LR35902 instruction set requires it.

### Raw Bytes

Status: Current code.

An audited escape hatch represented by `SectionItem::Raw` and emitted through
`Builder::raw`. Raw bytes are opaque to effect analysis and privileged by
default.

Raw bytes are not the normal way to write executable code.

### `Db`

Status: Current code.

A data directive for literal bytes. It carries provenance and is not an
instruction.

### `Dw`

Status: Current code.

A data directive for 16-bit words. F-A1 should define the encoded byte order
explicitly as little-endian for Game Boy data emission unless a future directive
states otherwise.

### Align

Status: Current code.

A directive that pads to an alignment. Before layout, its exact byte count is
unknown. After layout, it should become concrete padding bytes with provenance.

### Cycle Model

Status: Current module stub, open task T-A1.5.

A pure mapping from concrete `Instr` to M-cycle cost. Conditional control-flow
costs must distinguish taken and not-taken paths.

The cycle model is a static prediction. Calibration and drift reports are later
features.

### M-Cycle

Status: Hardware timing term.

A Game Boy machine cycle. Normal-speed LR35902 instructions are usually counted
in M-cycles; one M-cycle is four T-states/dots in the normal-speed framing used
by the plan.

Use M-cycles for scheduler and slice budgeting unless a type explicitly says
T-states or dots.

### T-State / Dot

Status: Hardware timing term.

Lower-level hardware timing unit used by Pan Docs rendering/timing discussions.
F-A1 cycle costs should not silently switch to T-states. If a conversion is
needed, name it.

### Listing / `.lst`

Status: Current module stub, open task T-A1.8.

A deterministic human-readable listing that maps addresses and bytes back to
instructions/data and provenance. It is a debug artifact, not a source program
or parser contract.

### ROM Builder

Status: RFC term, open task T-A1.9.

The code that assembles encoded sections into a `.gb` image, injects the
cartridge header, computes checksums, fills unused bytes, and pads to a
supported ROM size.

It should not be the owner of instruction encoding.

### `.gb`

Status: Output artifact term.

The final Game Boy ROM byte image.

### Cartridge Header

Status: Hardware/ROM term.

The required Game Boy header area around `$0100..=$014F`, including Nintendo
logo bytes, title, cartridge type, ROM/RAM size codes, and checksums.

F-A1's ROM builder owns generating this byte shape. Runtime builders should not
hand-roll header bytes except through audited helper paths.

### Bit-Stability

Status: Current project invariant.

The same logical inputs produce byte-identical outputs across runs. For F-A1,
this applies to encoded sections, ROM images, listings, and symbol maps.

### Determinism

Status: Current project invariant.

No output-visible behavior depends on hash-map iteration order, time, random
numbers, thread scheduling, environment, or host endianness. Determinism is
broader than bit-stability; it also covers stable diagnostics and reports.

## Provenance And Evidence Terms

### `InstrProvenance`

Status: Current code.

Structured origin metadata attached to every instruction and data directive.
It records `PlanningStage`, optional source value id, optional source op, and
optional note.

It is not a free-form comment substitute. It should survive through listing and
debug outputs.

### `PlanningStage`

Status: Current code.

The stable enum naming the compiler/planning stage that emitted or transformed
an assembly item. The explicit discriminants are part of the report/schema
contract.

### Source Node

Status: Current provenance term.

A compact adapter id pointing back to an originating compiler IR value without
making `gbf-asm` depend on later IR crates.

### Claim-To-Gate Matrix

Status: Current closure discipline.

A table mapping implementation claims to the tests or commands that prove them.
ASM closure should include this for non-trivial beads.

## Resolved F-A1 Terminology Decisions

### `LoweredSection` Versus `LegalizedSection`

Decision: use both terms, and keep them distinct.

Precision:

- `LoweredSection`: after `PreLayoutOp` lowering, not
  necessarily ready for bytes.
- `LegalizedSection`: ready for encoder.

Reason: layout, relaxation, final alignment, and thunk insertion are distinct
from `PreLayoutOp` lowering. Overloading `LoweredSection` makes it too easy to
feed unknown-width or unresolved symbolic items to the encoder.

## Terms That Need F-A1 Decision

### PreLayoutOp Versus LegalizationOp Classification

Decision needed.

The lowering order is named by the term:

- `PreLayoutOp`: lowers before layout.
- `LegalizationOp`: lowers during legalization after placement is known.

Current RFC direction: keep each structured op in the phase-named enum matching
whether its emitted form depends on final placement.

- Placement-independent variants become `PreLayoutOp`s and lower into
  `LoweredSection`.
- Placement-dependent variants become `LegalizationOp`s and lower into
  `LegalizedSection`.

Why that is attractive:

- common op intents can be simplified before layout;
- structured op generation is kept out of the encoder;
- F-A1 can test the seam before F-A4 production lowering exists;
- `FarCall` does not have to guess the callee bank before placement.

Why it might be wrong:

- layout now needs an explicit representation and size estimate for each
  `LegalizationOp`;
- split rules can be harder to explain than one lowering phase;
- if many current structured op variants become `LegalizationOp`s, the split buys
  little.

Decision criterion: if F-A4 confirms an op's emitted sequence does not depend
on final address or bank assignment, classify it as a `PreLayoutOp`. If it needs
final placement context, classify it as a `LegalizationOp`. `FarCall` is the
expected `LegalizationOp` case.

### BankLease Privilege

Decision needed.

Current code treats selected BankLease structured ops as privileged. That may force
normal generated sections to become privileged just to request legal bank
access. Better options:

- classify BankLease requests as `Normal` and rely on later reachability/lease
  validation; or
- add a narrow `BankLeaseRequest` privilege class that does not permit raw MBC
  writes.

Direct MBC writes must remain privileged either way.

### MBC5 `$6000..=$7FFF` Naming

Decision needed.

Current effect code has an `MbcRegisterClass::ModeSelect` catch-all for
`$6000..=$7FFF`. F-A2.3's bead names only RAMG, BANK1, BANK2, and RAMB for
MBC5. We need to reconcile whether this range is forbidden, unused/reserved, or
kept as a compatibility catch-all. The public name should not imply a real MBC5
mode register if that is not the intended hardware contract.

### Symbolic Branch Representation

Decision needed.

Current code has labels and concrete branch instructions, but not a durable
symbolic branch item. Branch relaxation needs target symbols. We should add a
`SectionItem::Branch` or equivalent rather than smuggling symbols into `Instr`.

## References

- `history/planv0.md` section "12. Backend (`AsmIR -> ReachabilityValidation -> PlacedRom -> EncodedRom`)".
- `history/planv0.md` section "Assembly eDSL (`gbf-asm`)".
- `history/planv0.md` section "Engineering rules".
- `history/rfcs/F-A1-gbf-asm.md`.
- `bd-ssm` and child beads T-A1.1 through T-A1.9.
- Pan Docs CPU instruction set: https://gbdev.io/pandocs/CPU_Instruction_Set.html
- gbdev opcode tables: https://gbdev.io/gb-opcodes/optables/classic
- Pan Docs MBC5: https://gbdev.io/pandocs/MBC5.html
- Pan Docs cartridge header: https://gbdev.io/pandocs/The_Cartridge_Header.html
